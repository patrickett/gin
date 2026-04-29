use crate::{prelude::*, ty_to_mlir};
use ast::{type_surface_mangle_name, Expr};
use typeck::{Ty, TyInfer};

impl<'c> Lower<'c> for WhenExpr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        // Infer the result type from the else arm, falling back to the first arm.
        let result_ty = {
            let body = self
                .arms
                .iter()
                .find_map(|a| {
                    if let WhenArm::Else(b) = a {
                        Some(b.as_ref())
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    self.arms.first().map(|a| match a {
                        WhenArm::Cond { body, .. }
                        | WhenArm::Is { body, .. }
                        | WhenArm::Else(body) => body.as_ref(),
                    })
                });
            body.map(|b| {
                let ty = b.infer_ty(&ctx.ty_env.infer_env(&*ctx.var_types.borrow()));
                ty_to_mlir(&ty, ctx.mlir)
            })
            .unwrap_or_else(|| ctx.mlir.i64())
        };

        if let Some(subject_expr) = &self.subject {
            let subject = subject_expr.lower(ctx, block, symtab)?;

            // Check if this is an optimized union (simple integer representation)
            let subject_ty =
                subject_expr.infer_ty(&ctx.ty_env.infer_env(&std::collections::HashMap::new()));

            let disc = match &subject_ty {
                Ty::Union { variants, .. }
                    if variants.iter().all(|(_, fields)| fields.is_empty()) =>
                {
                    // Optimized union: subject IS the discriminant
                    let variant_count = variants.len();

                    if variant_count == 2 {
                        // Convert i1 to i64
                        let loc = ctx.location();
                        let extend_op = match OperationBuilder::new("arith.extui", loc)
                            .add_operands(&[subject])
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
                    } else if variant_count <= 256 {
                        // Convert i8 to i64
                        let loc = ctx.location();
                        let extend_op = match OperationBuilder::new("arith.extsi", loc)
                            .add_operands(&[subject])
                            .add_results(&[ctx.mlir.i64()])
                            .build()
                        {
                            Ok(op) => op,
                            Err(e) => {
                                ctx.emit_internal(format!("extsi build failed: {e}"));
                                return None;
                            }
                        };
                        block.append_op(extend_op)
                    } else {
                        // Already i64
                        subject
                    }
                }
                _ => {
                    // Standard union: extract discriminant from struct at index 0
                    block.append_op(ctx.mlir.llvm_extractvalue(subject, 0, ctx.mlir.i64()))
                }
            };

            lower_pattern_when(ctx, block, symtab, subject, disc, &self.arms, result_ty)
        } else {
            lower_boolean_when(ctx, block, symtab, &self.arms, result_ty)
        }
    }
}

fn lower_boolean_when<'c>(
    ctx: &CodegenContext<'_, 'c>,
    outer_block: &BlockRef<'c, 'c>,
    symtab: &mut RuntimeSymbolTable<'c>,
    arms: &[WhenArm],
    result_ty: Type<'c>,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();

    let Some((head, tail)) = arms.split_first() else {
        return Some(outer_block.const_i64(ctx.mlir, 0));
    };

    match head {
        WhenArm::Else(body) => body.lower(ctx, outer_block, symtab),

        WhenArm::Is { .. } => {
            ctx.emit_internal("Is arm in boolean when — use 'when subject is ...' form");
            None
        }

        WhenArm::Cond { condition, body } => {
            let cond_val = condition.lower(ctx, outer_block, symtab)?;

            let value_producing = tail.iter().any(|a| matches!(a, WhenArm::Else(_)));
            let result_tys: Vec<Type<'c>> = if value_producing {
                vec![result_ty]
            } else {
                vec![]
            };

            let then_region = Region::new();
            {
                let blk = Block::new(&[]);
                then_region.append_block(blk);
                let blk_ref = then_region.first_block().unwrap();
                let val = body.lower(ctx, &blk_ref, &mut symtab.clone())?;
                if value_producing {
                    blk_ref.append_operation(scf_dialect::r#yield(&[val], loc));
                } else {
                    blk_ref.append_operation(scf_dialect::r#yield(&[], loc));
                }
            }

            let else_region = Region::new();
            if !tail.is_empty() {
                let blk = Block::new(&[]);
                else_region.append_block(blk);
                let blk_ref = else_region.first_block().unwrap();
                let val = lower_boolean_when(ctx, &blk_ref, &mut symtab.clone(), tail, result_ty)?;
                if value_producing {
                    blk_ref.append_operation(scf_dialect::r#yield(&[val], loc));
                } else {
                    blk_ref.append_operation(scf_dialect::r#yield(&[], loc));
                }
            }

            let if_op = scf_dialect::r#if(cond_val, &result_tys, then_region, else_region, loc);
            let result_op = outer_block.append_operation(if_op);

            if value_producing {
                Some(result_op.result(0).unwrap().into())
            } else {
                Some(outer_block.const_i64(ctx.mlir, 0))
            }
        }
    }
}

/// Lower a pattern-matching `when` expression.
///
/// `subject` is the full union value (always `union_type()`).
/// `disc` is the pre-extracted discriminant (`i64`) from `subject[0]`.
///
/// Pattern matching always yields an `i64` value; non-exhaustive matches
/// fall through to `const_i64(0)` as a default.
fn lower_pattern_when<'c>(
    ctx: &CodegenContext<'_, 'c>,
    outer_block: &BlockRef<'c, 'c>,
    symtab: &mut RuntimeSymbolTable<'c>,
    subject: Value<'c, 'c>,
    disc: Value<'c, 'c>,
    arms: &[WhenArm],
    result_ty: Type<'c>,
) -> Option<Value<'c, 'c>> {
    use internment::Intern;
    let loc = ctx.location();

    let Some((head, tail)) = arms.split_first() else {
        return Some(outer_block.const_i64(ctx.mlir, 0));
    };

    match head {
        WhenArm::Else(body) => body.lower(ctx, outer_block, symtab),

        WhenArm::Cond { .. } => {
            ctx.emit_internal("Cannot mix condition arms in a pattern match (when subject is ...)");
            None
        }

        WhenArm::Is { pattern, body } => {
            if !matches!(
                &pattern.0,
                Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. }
            ) {
                return None;
            }
            let variant_name =
                Intern::<String>::from_ref(type_surface_mangle_name(&pattern.0));
            // Note: unknown variant diagnostics are emitted by typeck; codegen just fails gracefully.
            let (_, expected_disc, _) = ctx.ty_env.lookup_variant(variant_name)?;

            let expected_val = outer_block.const_i64(ctx.mlir, expected_disc as i64);
            let cond =
                outer_block.append_op(ctx.mlir.build_cmpi(Predicates::EQ, disc, expected_val));

            let result_tys = vec![result_ty];

            // Build then-region: optionally bind payload fields, lower body.
            let then_region = Region::new();
            {
                let blk = Block::new(&[]);
                then_region.append_block(blk);
                let blk_ref = then_region.first_block().unwrap();
                let mut inner_symtab = symtab.clone();

                if let Expr::TypeGeneric { params, .. } = &pattern.0 {
                    let payload_fields = ctx
                        .ty_env
                        .lookup_variant(variant_name)
                        .map(|(_, _, f)| f)
                        .unwrap_or(&[]);
                    for (slot, (param_name, _)) in params.iter().enumerate() {
                        if param_name.as_str() == "_" {
                            continue;
                        }
                        let field_mlir_ty = payload_fields
                            .get(slot)
                            .map(|(_, ty)| ty_to_mlir(ty, ctx.mlir))
                            .unwrap_or_else(|| ctx.mlir.i64());
                        let extracted = blk_ref.append_op(ctx.mlir.llvm_extractvalue(
                            subject,
                            (slot + 1) as i64,
                            field_mlir_ty,
                        ));
                        inner_symtab.insert(param_name.as_str().to_string(), extracted);
                    }
                }

                let val = body.lower(ctx, &blk_ref, &mut inner_symtab)?;
                blk_ref.append_operation(scf_dialect::r#yield(&[val], loc));
            }

            // Build else-region: recurse on remaining arms.
            let else_region = Region::new();
            {
                let blk = Block::new(&[]);
                else_region.append_block(blk);
                let blk_ref = else_region.first_block().unwrap();
                let val = lower_pattern_when(
                    ctx,
                    &blk_ref,
                    &mut symtab.clone(),
                    subject,
                    disc,
                    tail,
                    result_ty,
                )?;
                blk_ref.append_operation(scf_dialect::r#yield(&[val], loc));
            }

            let if_op = scf_dialect::r#if(cond, &result_tys, then_region, else_region, loc);
            let result_op = outer_block.append_operation(if_op);
            Some(result_op.result(0).unwrap().into())
        }
    }
}
