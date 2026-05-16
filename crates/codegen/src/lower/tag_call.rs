use crate::prelude::*;
use crate::ty_to_mlir;
use ast::flow::ConstValue;
use ast::ty::Ty;
use internment::Intern;

/// Lower a typed tag call (from the typed expression arena) to an MLIR value.
/// This is the ExprId-based equivalent of the `Lower for TagCall` impl.
pub fn lower_typed_tag_call<'c>(
    ctx: &CodegenContext<'_, 'c>,
    _union_name: &Intern<String>,
    variant_name: &Intern<String>,
    discriminant: usize,
    args: &[Value<'c, 'c>],
    block: &BlockRef<'c, 'c>,
    _symtab: &mut ScopedSymbolTable<'c>,
) -> Option<Value<'c, 'c>> {
    // Try ConstUnion construction first.
    if let Some(Ty::ConstUnion { values, .. }) = ctx.lookup_tag(*variant_name) {
        let disc = if !args.is_empty() {
            discriminant as i64
        } else {
            ctx.emit_internal(format!(
                "ConstUnion '{}' requires exactly one argument",
                variant_name.as_str()
            ));
            return None;
        };
        return Some(super::emit_discriminant_constant(
            ctx,
            block,
            disc,
            values.len(),
        ));
    }

    // Try union variant construction.
    if let Some((union_name, disc, payload_fields)) = ctx.lookup_variant(*variant_name) {
        let union_ty = ctx.lookup_tag(union_name);
        let union_mlir_ty = union_ty
            .as_ref()
            .map(|ty| ty_to_mlir(ty, ctx.mlir))
            .unwrap_or_else(|| ctx.mlir.union_type());

        let is_optimized_simple = union_ty.as_ref().is_some_and(|ty| {
            matches!(ty, Ty::Union { variants, .. } if variants.iter().all(|(_, fields)| fields.is_empty()))
        });

        if is_optimized_simple {
            let variant_count = union_ty
                .as_ref()
                .and_then(|ty| {
                    if let Ty::Union { variants, .. } = ty {
                        Some(variants.len())
                    } else {
                        None
                    }
                })
                .unwrap_or(2);
            return Some(super::emit_discriminant_constant(
                ctx,
                block,
                disc as i64,
                variant_count,
            ));
        }

        // Standard union construction.
        let loc = ctx.location();
        let disc_val = block.const_i64(ctx.mlir, disc as i64);
        let mut val = block.append_op(ctx.mlir.llvm_undef(union_mlir_ty));
        val = block.append_op(ctx.mlir.llvm_insertvalue(val, disc_val, 0));

        for (slot, (arg, (_, field_ty))) in args.iter().zip(payload_fields.iter()).enumerate() {
            let field_mlir_ty = ty_to_mlir(field_ty, ctx.mlir);
            let coerced = if field_mlir_ty == ctx.mlir.i1() {
                let extend_op = match OperationBuilder::new("arith.extui", loc)
                    .add_operands(&[*arg])
                    .add_results(&[ctx.mlir.i64()])
                    .build()
                {
                    Ok(op) => op,
                    Err(e) => {
                        ctx.emit_internal(format!("extui build failed: {e}"));
                        return None;
                    }
                };
                block.append_op(extend_op)
            } else {
                *arg
            };
            val = block.append_op(ctx.mlir.llvm_insertvalue(val, coerced, (slot + 1) as i64));
        }
        return Some(val);
    }

    None
}

impl<'c> Lower<'c> for TagCall {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        // Try ConstUnion construction first (e.g. LogLevel('debug')).
        if let Some(Ty::ConstUnion { values, .. }) = ctx.lookup_tag(self.name) {
            if let Some(arg) = self.args.first() {
                let arg_lit = match &arg.value {
                    ast::Expr::Lit(lit) => lit,
                    _ => {
                        // TODO: Support non-literal ConstUnion construction (e.g.
                        // passing a runtime string through pattern matching). This
                        // would need a runtime string comparison against each value
                        // to find the matching discriminant.
                        ctx.emit_internal(format!(
                            "ConstUnion '{}' requires a literal argument",
                            self.name.as_str()
                        ));
                        return None;
                    }
                };
                let disc = ConstValue::find_discriminant(arg_lit, values)? as i64;
                return Some(super::emit_discriminant_constant(
                    ctx,
                    block,
                    disc,
                    values.len(),
                ));
            }
            ctx.emit_internal(format!(
                "ConstUnion '{}' requires exactly one argument",
                self.name.as_str()
            ));
            return None;
        }

        // Try union variant construction first.
        if let Some((union_name, discriminant, payload_fields)) = ctx.lookup_variant(self.name) {
            let union_ty = ctx.lookup_tag(union_name);
            let union_mlir_ty = union_ty
                .as_ref()
                .map(|ty| ty_to_mlir(ty, ctx.mlir))
                .unwrap_or_else(|| ctx.mlir.union_type());

            // Check if this is an optimized union with no fields (like Bool)
            let is_optimized_simple = union_ty.as_ref().is_some_and(|ty| {
                matches!(ty, Ty::Union { variants, .. } if variants.iter().all(|(_, fields)| fields.is_empty()))
            });

            if is_optimized_simple {
                let variant_count = union_ty
                    .as_ref()
                    .and_then(|ty| {
                        if let Ty::Union { variants, .. } = ty {
                            Some(variants.len())
                        } else {
                            None
                        }
                    })
                    .unwrap_or(2);
                return Some(super::emit_discriminant_constant(
                    ctx,
                    block,
                    discriminant as i64,
                    variant_count,
                ));
            }

            // Standard union construction with struct representation
            let disc_val = block.const_i64(ctx.mlir, discriminant as i64);
            let mut val = block.append_op(ctx.mlir.llvm_undef(union_mlir_ty));
            val = block.append_op(ctx.mlir.llvm_insertvalue(val, disc_val, 0));
            for (slot, (arg, (_, field_ty))) in
                self.args.iter().zip(payload_fields.iter()).enumerate()
            {
                let lowered = arg.lower(ctx, block, symtab)?;
                let field_mlir_ty = ty_to_mlir(field_ty, ctx.mlir);
                let coerced = if field_mlir_ty == ctx.mlir.i1() {
                    let loc = ctx.location();
                    let extend_op = match OperationBuilder::new("arith.extui", loc)
                        .add_operands(&[lowered])
                        .add_results(&[ctx.mlir.i64()])
                        .build()
                    {
                        Ok(op) => op,
                        Err(e) => {
                            ctx.emit_internal(format!("extui build failed: {e}"));
                            return None;
                        }
                    };
                    block.append_op(extend_op)
                } else {
                    lowered
                };
                val = block.append_op(ctx.mlir.llvm_insertvalue(val, coerced, (slot + 1) as i64));
            }
            return Some(val);
        }

        // Fall back to record construction.
        // Note: unknown tag diagnostics are emitted by typeck; codegen just fails gracefully.
        let record_ty = ctx.lookup_tag(self.name).cloned()?;

        match &record_ty {
            Ty::Record { .. } => {
                let fields = record_ty.record_fields_sorted();
                let struct_type = ty_to_mlir(&record_ty, ctx.mlir);
                let mut val = block.append_op(ctx.mlir.llvm_undef(struct_type));

                // Named construction: `Tag(field: val, ...)` — args parse as Bind expressions.
                let is_named = self.args.iter().any(|a| matches!(&a.value, Expr::Bind(_)));
                if is_named {
                    for arg in &self.args {
                        let Expr::Bind(bind) = &arg.value else {
                            ctx.emit_internal(format!(
                                "Mixed named/positional args in record '{}' constructor",
                                self.name.as_str()
                            ));
                            return None;
                        };
                        let BindValue::Expr(value_expr) = bind.value() else {
                            ctx.emit_internal("Named record arg must be a simple expression");
                            return None;
                        };
                        let field_name = bind.name();
                        let idx = match fields.iter().position(|(fname, _)| **fname == field_name) {
                            Some(i) => i,
                            None => {
                                ctx.emit_internal(format!(
                                    "No field '{}' on record '{}'",
                                    field_name.as_str(),
                                    self.name.as_str()
                                ));
                                return None;
                            }
                        };
                        let arg_val = value_expr.lower(ctx, block, symtab)?;
                        val = block.append_op(ctx.mlir.llvm_insertvalue(val, arg_val, idx as i64));
                    }
                } else {
                    // Positional construction: `Tag(val1, val2, ...)` — args by layout order.
                    for (i, _) in fields.iter().enumerate() {
                        let arg = match self.args.get(i) {
                            Some(a) => a,
                            None => {
                                ctx.emit_internal(format!(
                                    "Not enough args for record '{}': expected {}, got {}",
                                    self.name.as_str(),
                                    fields.len(),
                                    self.args.len()
                                ));
                                return None;
                            }
                        };
                        let arg_val = arg.lower(ctx, block, symtab)?;
                        val = block.append_op(ctx.mlir.llvm_insertvalue(val, arg_val, i as i64));
                    }
                }
                Some(val)
            }
            _ => None,
        }
    }
}
