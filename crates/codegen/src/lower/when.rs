use crate::{prelude::*, ty_to_mlir};
use ast::TyInfer;
use ast::ty::Ty;
use ast::type_surface_mangle_name;

impl<'c> Lower<'c> for WhenExpr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        // Infer the result type from the else arm, falling back to the first arm.
        let result_ty = {
            let body = self
                .arms
                .iter()
                .find_map(|a| {
                    if let WhenArm::Else(b, _) = a {
                        Some(b.as_ref())
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    self.arms.first().map(|a| match a {
                        WhenArm::Cond { body, .. }
                        | WhenArm::Is { body, .. }
                        | WhenArm::Else(body, _) => body.as_ref(),
                    })
                });
            body.map(|b| {
                let ty = b.infer_ty(&ctx.infer_env(&*ctx.var_types.borrow()));
                ty_to_mlir(&ty, ctx.mlir)
            })
            .unwrap_or_else(|| ctx.mlir.i64())
        };

        if let Some(subject_expr) = &self.subject {
            let subject = subject_expr.lower(ctx, block, symtab)?;

            // Check if this is an optimized union (simple integer representation)
            let subject_ty = subject_expr.infer_ty(&ctx.infer_env(&*ctx.var_types.borrow()));

            let disc = match &subject_ty {
                Ty::ConstUnion { values, .. } => {
                    super::emit_discriminant_extend(ctx, block, subject, values.len())
                }
                Ty::Union { variants, .. }
                    if variants.iter().all(|(_, fields)| fields.is_empty()) =>
                {
                    super::emit_discriminant_extend(ctx, block, subject, variants.len())
                }
                _ => block.append_op(ctx.mlir.llvm_extractvalue(subject, 0, ctx.mlir.i64())),
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
    symtab: &mut ScopedSymbolTable<'c>,
    arms: &[WhenArm],
    result_ty: Type<'c>,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();

    let Some((head, tail)) = arms.split_first() else {
        return Some(outer_block.const_i64(ctx.mlir, 0));
    };

    match head {
        WhenArm::Else(body, _) => body.lower(ctx, outer_block, symtab),

        WhenArm::Is { .. } => {
            ctx.emit_internal("Is arm in boolean when — use 'when subject is ...' form");
            None
        }

        WhenArm::Cond {
            condition, body, ..
        } => {
            let cond_val = condition.lower(ctx, outer_block, symtab)?;

            let value_producing = tail.iter().any(|a| matches!(a, WhenArm::Else(_, _)));
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
    symtab: &mut ScopedSymbolTable<'c>,
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
        WhenArm::Else(body, _) => body.lower(ctx, outer_block, symtab),

        WhenArm::Cond { .. } => {
            ctx.emit_internal("Cannot mix condition arms in a pattern match (when subject is ...)");
            None
        }

        WhenArm::Is { pattern, body, .. } => {
            let expected_disc: i64 = {
                let variant_name =
                    Intern::<String>::from_ref(type_surface_mangle_name(&pattern.value));
                let (_, disc, _) = ctx.lookup_variant(variant_name)?;
                disc as i64
            };

            let expected_val = outer_block.const_i64(ctx.mlir, expected_disc);
            let cond =
                outer_block.append_op(ctx.mlir.build_cmpi(Predicates::EQ, disc, expected_val));

            let result_tys = vec![result_ty];

            let then_region = Region::new();
            {
                let blk = Block::new(&[]);
                then_region.append_block(blk);
                let blk_ref = then_region.first_block().unwrap();
                let mut inner_symtab = symtab.clone();

                super::bind_pattern_payload_fields(
                    ctx,
                    &blk_ref,
                    &pattern.value,
                    subject,
                    &mut inner_symtab,
                );

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
