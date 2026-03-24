use crate::codegen::{addressof_string_global, prelude::*, ty_to_mlir};
use crate::diagnostic::codegen::CodegenSymptom;
use crate::parse::delimited_list;
use crate::prelude::*;
use crate::typeck::Ty;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnCall {
    pub path: ModPath,
    pub args: Option<Vec<Expr>>,
}

pub fn fn_call<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, FnCall, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    let args = delimited_list(ParenOpen, expr, Comma, ParenClose).or_not();

    // Tag-rooted path (e.g. `Byte.new`, `Int.to_string`) takes priority so that
    // `Byte.new(...)` is not swallowed by the AnonymousTag arm first.
    let tag_fn = tag_path()
        .then(args.clone())
        .then_ignore(just(Newline).or_not())
        .map(|(path, args)| FnCall { path, args });

    let id_fn = path()
        .then(args)
        .then_ignore(just(Newline).or_not())
        .map(|(path, args)| FnCall { path, args });

    choice((tag_fn, id_fn))
}

impl<'c> Lower<'c> for FnCall {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        if !self.path.segments.is_empty() {
            let root = self.path.root.as_str();
            if let Some(ty) = ctx.var_types.borrow().get(root).cloned() {
                // Unwrap one level of Ptr/Ref for auto-deref field access.
                let record_ty = match &ty {
                    Ty::Ptr { inner } | Ty::Ref { inner } if matches!(inner.as_ref(), Ty::Record { .. }) => {
                        inner.as_ref().clone()
                    }
                    other => other.clone(),
                };

                match (&record_ty, &self.args) {
                    // `p.x` — field access; `p.dist` — no-arg method call.
                    (Ty::Record { name: type_name, .. }, None) => {
                        let segment = self.path.segments.last().unwrap();
                        let is_field = record_ty
                            .record_fields_sorted()
                            .iter()
                            .any(|(name, _)| name.as_str() == segment.as_str());
                        if is_field {
                            return lower_field_access(
                                ctx,
                                block,
                                symtab,
                                root,
                                &self.path.segments,
                                ty,
                            );
                        }
                        let mangled = IStr::new(format!(
                            "{}.{}",
                            type_name.as_str(),
                            segment.as_str()
                        ));
                        let self_val = symtab
                            .get(root)
                            .copied()
                            .ok_or_else(|| CodegenSymptom::Internal(format!("Unknown variable '{root}'")))?;
                        let return_type = ctx
                            .ty_env
                            .fn_return_ty(&mangled)
                            .map(|t| ty_to_mlir(t, ctx.mlir))
                            .unwrap_or_else(|| ctx.mlir.i64());
                        return Ok(block.call(ctx.mlir, mangled.as_str(), &[self_val], return_type));
                    }
                    // Method call: `p.method(args)` or `self.method(args)`.
                    (Ty::Record { name: type_name, .. } | Ty::Union { name: type_name, .. }, Some(arg_exprs)) => {
                        let method = self.path.segments.last().unwrap();
                        let mangled = IStr::new(format!(
                            "{}.{}",
                            type_name.as_str(),
                            method.as_str()
                        ));
                        let self_val = symtab
                            .get(root)
                            .copied()
                            .ok_or_else(|| CodegenSymptom::Internal(format!("Unknown variable '{root}'")))?;
                        let mut args = vec![self_val];
                        for expr in arg_exprs {
                            args.push(expr.lower(ctx, block, symtab)?);
                        }
                        let return_type = ctx
                            .ty_env
                            .fn_return_ty(&mangled)
                            .map(|t| ty_to_mlir(t, ctx.mlir))
                            .unwrap_or_else(|| ctx.mlir.i64());
                        return Ok(block.call(ctx.mlir, mangled.as_str(), &args, return_type));
                    }
                    _ => {
                        // Primitive type method dispatch: Int, I128, Float, Bool, etc.
                        let prim_name = match &record_ty {
                            Ty::Int(128) => Some("I128"),
                            Ty::Int(64) => Some("Int"),
                            Ty::Int(32) => Some("I32"),
                            Ty::Int(16) => Some("I16"),
                            Ty::Int(8) => Some("Byte"),
                            Ty::Float => Some("Float"),
                            Ty::Bool => Some("Bool"),
                            _ => None,
                        };
                        if let Some(type_name) = prim_name {
                            let method = self.path.segments.last().unwrap();
                            let mangled = IStr::new(format!("{type_name}.{}", method.as_str()));
                            // Load value from alloca slot if mutable; use raw SSA value otherwise.
                            let self_val = if ctx.mutable_slots.borrow().contains(root) {
                                let elem_mlir_ty = ty_to_mlir(&record_ty, ctx.mlir);
                                let loc = ctx.location();
                                let ptr = symtab.get(root).copied().ok_or_else(|| {
                                    CodegenSymptom::Internal(format!("Unknown variable '{root}'"))
                                })?;
                                block.load_typed(ctx.mlir, ptr, elem_mlir_ty, loc)?
                            } else {
                                symtab.get(root).copied().ok_or_else(|| {
                                    CodegenSymptom::Internal(format!("Unknown variable '{root}'"))
                                })?
                            };
                            let mut args = vec![self_val];
                            if let Some(arg_exprs) = &self.args {
                                for expr in arg_exprs {
                                    args.push(expr.lower(ctx, block, symtab)?);
                                }
                            }
                            let return_type = ctx
                                .ty_env
                                .fn_return_ty(&mangled)
                                .map(|t| ty_to_mlir(t, ctx.mlir))
                                .unwrap_or_else(|| ctx.mlir.i64());
                            return Ok(block.call(ctx.mlir, mangled.as_str(), &args, return_type));
                        }
                    }
                }
            }
        }

        let func_name = if self.path.segments.is_empty() {
            self.path.root
        } else {
            let segs: Vec<&str> = self.path.segments.iter().map(|s| s.as_str()).collect();
            IStr::new(format!("{}.{}", self.path.root, segs.join(".")))
        };

        if func_name.as_str() == "syscall" {
            return self.lower_syscall_call(ctx, block, symtab);
        }

        if func_name.as_str() == "float_bits" {
            let arg = self.args.as_ref()
                .and_then(|a| a.first())
                .ok_or_else(|| CodegenSymptom::Internal("float_bits requires one argument".into()))?;
            let val = arg.lower(ctx, block, symtab)?;
            let loc = ctx.location();
            return OperationBuilder::new("llvm.bitcast", loc)
                .add_operands(&[val])
                .add_results(&[ctx.mlir.i64()])
                .build()
                .map(|op| block.append_op(op))
                .map_err(|e| CodegenSymptom::Internal(format!("llvm.bitcast: {e}")));
        }

        // Global constant array — return a pointer via addressof.
        if ctx.global_const_elems.borrow().contains_key(func_name.as_str()) {
            return addressof_string_global(ctx.mlir, block, func_name.as_str());
        }

        if let Some(&ptr) = symtab.get(func_name.as_str()) {
            if ctx.mutable_slots.borrow().contains(func_name.as_str()) {
                // Mutable variable — load from alloca slot.
                let ty = ctx
                    .var_types
                    .borrow()
                    .get(func_name.as_str())
                    .cloned()
                    .unwrap_or(crate::typeck::Ty::Int(64));
                let elem_mlir_ty = ty_to_mlir(&ty, ctx.mlir);
                let loc = ctx.location();
                return block.load_typed(ctx.mlir, ptr, elem_mlir_ty, loc);
            }
            return Ok(ptr);
        }

        // A bind (no-param `:=` definition) with explicit call args is a type error.
        // A bare reference with no args is fine — the bind was compiled as a 0-arg function.
        if let Some(symbol) = ctx.symbol_table.get(&func_name)
            && symbol.is_bind()
            && self.args.is_some()
        {
            return Err(CodegenSymptom::Internal(format!(
                "Cannot call '{func_name}': it is a value definition (bind), not a function"
            )));
        }

        let mut args = Vec::new();
        if let Some(arg_exprs) = &self.args {
            for arg_expr in arg_exprs {
                args.push(arg_expr.lower(ctx, block, symtab)?);
            }
        }

        let ret_ty = ctx.ty_env.fn_return_ty(&func_name).cloned();
        if matches!(ret_ty, Some(Ty::Unit)) {
            block.call_void(ctx.mlir, func_name.as_str(), &args);
            return Ok(block.unit_value(ctx));
        }
        let return_type = ret_ty
            .map(|ty| ty_to_mlir(&ty, ctx.mlir))
            .unwrap_or_else(|| ctx.mlir.i64());
        Ok(block.call(ctx.mlir, func_name.as_str(), &args, return_type))
    }
}

fn lower_field_access<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    symtab: &RuntimeSymbolTable<'c>,
    root: &str,
    segments: &[IStr],
    ty: Ty,
) -> Result<Value<'c, 'c>, CodegenSymptom> {
    let slot = symtab
        .get(root)
        .copied()
        .ok_or_else(|| CodegenSymptom::Internal(format!("Unknown variable '{root}'")))?;

    // Auto-deref: unwrap one level of Ptr/Ref to reach the record type.
    let (mut ty, mut val) = match ty {
        Ty::Ptr { inner } | Ty::Ref { inner } => {
            let record_ty = *inner;
            let struct_mlir_ty = ty_to_mlir(&record_ty, ctx.mlir);
            let loc = ctx.location();
            let loaded = block
                .load_typed(ctx.mlir, slot, struct_mlir_ty, loc)
                .map_err(|e| CodegenSymptom::Internal(format!("auto-deref load: {e:?}")))?;
            (record_ty, loaded)
        }
        ref record_ty @ Ty::Record { .. } => {
            // Mutable record variable: slot holds alloca ptr → load struct value first.
            if ctx.mutable_slots.borrow().contains(root) {
                let struct_mlir_ty = ty_to_mlir(record_ty, ctx.mlir);
                let loc = ctx.location();
                let loaded = block
                    .load_typed(ctx.mlir, slot, struct_mlir_ty, loc)
                    .map_err(|e| CodegenSymptom::Internal(format!("record load: {e:?}")))?;
                (record_ty.clone(), loaded)
            } else {
                (record_ty.clone(), slot)
            }
        }
        other => (other, slot),
    };

    for seg in segments {
        let fields = ty.record_fields_sorted();
        let (field_idx, next_ty) = fields
            .iter()
            .enumerate()
            .find_map(|(i, (fname, fty))| {
                if fname.as_str() == seg.as_str() {
                    Some((i, (*fty).clone()))
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                CodegenSymptom::Internal(format!(
                    "No field '{}' on record type",
                    seg.as_str()
                ))
            })?;
        let result_mlir_ty = ty_to_mlir(&next_ty, ctx.mlir);
        val = block.append_op(ctx.mlir.llvm_extractvalue(val, field_idx as i64, result_mlir_ty));
        ty = next_ty;
    }

    Ok(val)
}

impl FnCall {
    fn lower_syscall_call<'c>(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        let loc = ctx.location();
        let zero = block.const_i64(ctx.mlir, 0);
        let empty: Vec<Expr> = Vec::new();
        let arg_exprs = self.args.as_deref().unwrap_or(&empty);

        // Lower up to 6 args, padding missing ones with 0.
        let mut operands: Vec<Value<'c, 'c>> = Vec::with_capacity(6);
        for i in 0..6 {
            let val = if i < arg_exprs.len() {
                arg_exprs[i].lower(ctx, block, symtab)?
            } else {
                zero
            };
            operands.push(val);
        }

        // aarch64: syscall number goes in x16 (macOS) or x8 (Linux).
        // Args go in x0–x4 (a0 is tied to the x0 output register).
        // Result comes back in x0.
        #[cfg(target_os = "macos")]
        let (asm_str, num_reg) = ("svc #0x80", "x16");
        #[cfg(not(target_os = "macos"))]
        let (asm_str, num_reg) = ("svc #0", "x8");

        let constraints = format!(
            "={{x0}},{{{num_reg}}},0,{{x1}},{{x2}},{{x3}},{{x4}},~{{memory}}"
        );

        let bool_true =
            IntegerAttribute::new(IntegerType::new(ctx.mlir, 1).into(), 1).into();

        let asm_op = OperationBuilder::new("llvm.inline_asm", loc)
            .add_attributes(&[
                (
                    Identifier::new(ctx.mlir, "asm_string"),
                    StringAttribute::new(ctx.mlir, asm_str).into(),
                ),
                (
                    Identifier::new(ctx.mlir, "constraints"),
                    StringAttribute::new(ctx.mlir, &constraints).into(),
                ),
                (
                    Identifier::new(ctx.mlir, "has_side_effects"),
                    bool_true,
                ),
            ])
            .add_operands(&operands)
            .add_results(&[ctx.mlir.i64()])
            .build()
            .map_err(|e| CodegenSymptom::Internal(format!("llvm.inline_asm: {e}")))?;

        Ok(block.append_op(asm_op))
    }
}
