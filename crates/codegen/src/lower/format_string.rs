use crate::prelude::*;

impl<'a, 'c> CodegenContext<'a, 'c> {
    fn memcpy_op<'c2>(
        ctx: &'c2 Context,
        dst: Value<'c2, 'c2>,
        src: Value<'c2, 'c2>,
        len: Value<'c2, 'c2>,
        loc: Location<'c2>,
    ) -> Option<Operation<'c2>> {
        OperationBuilder::new("llvm.intr.memcpy", loc)
            .add_attributes(&[(
                Identifier::new(ctx, "isVolatile"),
                IntegerAttribute::new(IntegerType::new(ctx, 1).into(), 0).into(),
            )])
            .add_operands(&[dst, src, len])
            .build()
            .ok()
    }
    /// Lower a typed-arena format string expression.
    pub(crate) fn lower_typed_format_string(
        &self,
        fs: &ast::FormatString,
        block: &BlockRef<'c, 'c>,
        _symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = self.location();

        // 1. Lower each part to a (ptr, len) pair.
        let mut parts: Vec<(Value<'c, 'c>, Value<'c, 'c>)> = Vec::new();

        for part in &fs.parts {
            match part {
                ast::FormatPart::Text(s) => {
                    if s.is_empty() {
                        continue;
                    }
                    let name = self.register_string(s);
                    let ptr = match self.addressof_string_global(block, &name) {
                        Some(p) => p,
                        None => {
                            self.emit_internal(format!(
                                "failed to get address of string global '{name}'"
                            ));
                            return None;
                        }
                    };
                    let len = block.const_i64(self.mlir, s.len() as i64);
                    parts.push((ptr, len));
                }
                ast::FormatPart::Expr(_e, _) => {
                    // In the typed-arena path, expressions are lowered via ExprId. The format
                    // string stores its parts as AST nodes; this path lower each sub-expression
                    // as an inline expression using the typed-arena dispatcher.
                    // For now, format string expressions fall through to the old Lower trait path.
                    return None;
                }
            }
        }

        if parts.is_empty() {
            let undef = block.append_op(self.mlir.llvm_undef(self.mlir.string_type()));
            let zero = block.const_i64(self.mlir, 0);
            return Some(block.append_op(self.mlir.llvm_insertvalue(undef, zero, 1)));
        }

        // 2. Sum all lengths.
        let zero = block.const_i64(self.mlir, 0);
        let total_len = parts.iter().fold(zero, |acc, (_, len)| {
            block.append_op(
                self.mlir
                    .build_binop("arith.addi", acc, *len, self.mlir.i64()),
            )
        });

        // 3. Allocate a stack buffer of total_len bytes.
        let buf = block.append_op(melior::dialect::llvm::alloca(
            self.mlir,
            total_len,
            self.mlir.llvm_ptr(),
            loc,
            melior::dialect::llvm::AllocaOptions::default().elem_type(Some(TypeAttribute::new(
                IntegerType::new(self.mlir, 8).into(),
            ))),
        ));

        // 4. Copy each part into the buffer at increasing offsets.
        let mut cur_offset = block.const_i64(self.mlir, 0);
        for (src_ptr, len) in &parts {
            let dst_ptr = block.gep_i8(self, buf, cur_offset, loc)?;
            let memcpy_operation = Self::memcpy_op(self.mlir, dst_ptr, *src_ptr, *len, loc)?;
            block.append_operation(memcpy_operation);
            cur_offset = block.append_op(self.mlir.build_binop(
                "arith.addi",
                cur_offset,
                *len,
                self.mlir.i64(),
            ));
        }

        // 5. Return {buf, total_len} as a string fat pointer.
        let undef = block.append_op(self.mlir.llvm_undef(self.mlir.string_type()));
        let with_ptr = block.append_op(self.mlir.llvm_insertvalue(undef, buf, 0));
        Some(block.append_op(self.mlir.llvm_insertvalue(with_ptr, total_len, 1)))
    }
}
