use crate::{addressof_string_global, prelude::*};
use typeck::{Ty, TyInfer};

fn to_string_fn_name(ty: &Ty) -> String {
    match ty {
        Ty::Int { .. } => "Int.to_string".to_string(),
        Ty::Float { .. } => "Float.to_string".to_string(),
        Ty::Bool => "Bool.to_string".to_string(),
        Ty::Unit => "Unit.to_string".to_string(),
        Ty::Opaque(name) | Ty::Record { name, .. } | Ty::Union { name, .. } => {
            format!("{}.to_string", name.as_str())
        }
        Ty::Array { .. } | Ty::Ptr { .. } | Ty::Ref { .. } => "Ptr.to_string".to_string(),
        Ty::Tuple(_) => "Int.to_string".to_string(),
    }
}

fn memcpy_op<'c>(
    ctx: &'c Context,
    dst: Value<'c, 'c>,
    src: Value<'c, 'c>,
    len: Value<'c, 'c>,
    loc: Location<'c>,
) -> Option<Operation<'c>> {
    OperationBuilder::new("llvm.intr.memcpy", loc)
        .add_attributes(&[(
            Identifier::new(ctx, "isVolatile"),
            IntegerAttribute::new(IntegerType::new(ctx, 1).into(), 0).into(),
        )])
        .add_operands(&[dst, src, len])
        .build()
        .ok()
}

impl<'c> Lower<'c> for FormatString {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = ctx.location();

        // 1. Lower each part to a (ptr, len) pair.
        let mut parts: Vec<(Value<'c, 'c>, Value<'c, 'c>)> = Vec::new();

        for part in &self.parts {
            match part {
                FormatPart::Text(s) => {
                    if s.is_empty() {
                        continue;
                    }
                    let name = ctx.register_string(s);
                    let ptr = match addressof_string_global(ctx.mlir, block, &name) {
                        Some(p) => p,
                        None => {
                            ctx.emit_internal(format!(
                                "failed to get address of string global '{}'",
                                name
                            ));
                            return None;
                        }
                    };
                    let len = block.const_i64(ctx.mlir, s.len() as i64);
                    parts.push((ptr, len));
                }
                FormatPart::Expr(e) => {
                    let val = e.lower(ctx, block, symtab)?;
                    let ty = e.infer_ty(&ctx.ty_env.infer_env(&std::collections::HashMap::new()));
                    let fn_name = to_string_fn_name(&ty);
                    let loc = ctx.location();
                    let str_val =
                        block.call(ctx.mlir, &fn_name, &[val], ctx.mlir.string_type(), loc);
                    let ptr = block.append_op(ctx.mlir.llvm_extractvalue(
                        str_val,
                        0,
                        ctx.mlir.llvm_ptr(),
                    ));
                    let len =
                        block.append_op(ctx.mlir.llvm_extractvalue(str_val, 1, ctx.mlir.i64()));
                    parts.push((ptr, len));
                }
            }
        }

        if parts.is_empty() {
            let undef = block.append_op(ctx.mlir.llvm_undef(ctx.mlir.string_type()));
            let zero = block.const_i64(ctx.mlir, 0);
            return Some(block.append_op(ctx.mlir.llvm_insertvalue(undef, zero, 1)));
        }

        // 2. Sum all lengths.
        let zero = block.const_i64(ctx.mlir, 0);
        let total_len = parts.iter().fold(zero, |acc, (_, len)| {
            block.append_op(
                ctx.mlir
                    .build_binop("arith.addi", acc, *len, ctx.mlir.i64()),
            )
        });

        // 3. Allocate a stack buffer of total_len bytes.
        let buf = block.append_op(melior::dialect::llvm::alloca(
            ctx.mlir,
            total_len,
            ctx.mlir.llvm_ptr(),
            loc,
            melior::dialect::llvm::AllocaOptions::default().elem_type(Some(TypeAttribute::new(
                IntegerType::new(ctx.mlir, 8).into(),
            ))),
        ));

        // 4. Copy each part into the buffer at increasing offsets.
        let mut cur_offset = block.const_i64(ctx.mlir, 0);
        for (src_ptr, len) in &parts {
            let dst_ptr = block.gep_i8(ctx, buf, cur_offset, loc)?;
            let memcpy_operation = match memcpy_op(ctx.mlir, dst_ptr, *src_ptr, *len, loc) {
                Some(op) => op,
                None => {
                    ctx.emit_internal("memcpy operation failed in format string");
                    return None;
                }
            };
            block.append_operation(memcpy_operation);
            cur_offset = block.append_op(ctx.mlir.build_binop(
                "arith.addi",
                cur_offset,
                *len,
                ctx.mlir.i64(),
            ));
        }

        // 5. Return {buf, total_len} as a string fat pointer.
        let undef = block.append_op(ctx.mlir.llvm_undef(ctx.mlir.string_type()));
        let with_ptr = block.append_op(ctx.mlir.llvm_insertvalue(undef, buf, 0));
        Some(block.append_op(ctx.mlir.llvm_insertvalue(with_ptr, total_len, 1)))
    }
}
