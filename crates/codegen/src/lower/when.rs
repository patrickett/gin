use super::lower_typed_expr;
use crate::{prelude::*, ty_to_mlir};
use ast::type_surface_mangle_name;
use internment::Intern;
use typed_ast::TypedWhenArm;

/// Lower a typed-arena `when` expression (runtime path).
pub(crate) fn lower_typed_when<'c>(
    ctx: &CodegenContext<'_, 'c>,
    when_expr: &typed_ast::TypedWhenExpr,
    block: &BlockRef<'c, 'c>,
    symtab: &mut ScopedSymbolTable<'c>,
) -> Option<Value<'c, 'c>> {
    let _loc = ctx.location();
    let typed_ast = ctx.typed_ast?;

    // Determine result type from else arm or first arm.
    let result_mlir = {
        let else_ret = when_expr.arms.iter().find_map(|a| {
            if let TypedWhenArm::Else(body, _) = a {
                typed_ast
                    .expr(*body)
                    .map(|e| ty_to_mlir(e.ty, ctx.mlir, ctx.typed_ast.unwrap(), ctx.trait_registry))
            } else {
                None
            }
        });
        else_ret
            .or_else(|| {
                when_expr.arms.first().and_then(|a| {
                    let body_id = match a {
                        TypedWhenArm::Cond { body, .. }
                        | TypedWhenArm::Is { body, .. }
                        | TypedWhenArm::Else(body, _) => *body,
                    };
                    typed_ast.expr(body_id).map(|e| {
                        ty_to_mlir(e.ty, ctx.mlir, ctx.typed_ast.unwrap(), ctx.trait_registry)
                    })
                })
            })
            .unwrap_or_else(|| ctx.mlir.i64())
    };

    match when_expr.subject {
        Some(subject_id) => {
            let subject = lower_typed_expr(ctx, subject_id, block, symtab)?;
            if let Some(expr_ref) = typed_ast.expr(subject_id) {
                let disc = match &expr_ref.ty {
                    ty if ty.union_literal_values().is_some() => {
                        let n = ty.union_literal_values().unwrap().len();
                        super::emit_discriminant_extend(ctx, block, subject, n)
                    }
                    typed_ast::ty::Ty::Union { variants, .. }
                        if variants.iter().all(|(_, fields)| fields.is_empty()) =>
                    {
                        super::emit_discriminant_extend(ctx, block, subject, variants.len())
                    }
                    _ => block.append_op(ctx.mlir.llvm_extractvalue(subject, 0, ctx.mlir.i64())),
                };
                lower_typed_pattern_when(
                    ctx,
                    block,
                    symtab,
                    subject,
                    disc,
                    &when_expr.arms,
                    result_mlir,
                )
            } else {
                None
            }
        }
        None => lower_typed_boolean_when(ctx, block, symtab, &when_expr.arms, result_mlir),
    }
}

fn lower_typed_boolean_when<'c>(
    ctx: &CodegenContext<'_, 'c>,
    outer_block: &BlockRef<'c, 'c>,
    symtab: &mut ScopedSymbolTable<'c>,
    arms: &[TypedWhenArm],
    result_ty: Type<'c>,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();

    let Some((head, tail)) = arms.split_first() else {
        return Some(outer_block.const_i64(ctx.mlir, 0));
    };

    match head {
        TypedWhenArm::Else(body, _) => lower_typed_expr(ctx, *body, outer_block, symtab),
        TypedWhenArm::Is { .. } => {
            ctx.emit_internal("Is arm in boolean when — use 'when subject is ...' form");
            None
        }
        TypedWhenArm::Cond {
            condition, body, ..
        } => {
            let cond_val = lower_typed_expr(ctx, *condition, outer_block, symtab)?;
            let value_producing = tail.iter().any(|a| matches!(a, TypedWhenArm::Else(..)));
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
                let val = lower_typed_expr(ctx, *body, &blk_ref, &mut symtab.clone())?;
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
                let val =
                    lower_typed_boolean_when(ctx, &blk_ref, &mut symtab.clone(), tail, result_ty)?;
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

fn lower_typed_pattern_when<'c>(
    ctx: &CodegenContext<'_, 'c>,
    outer_block: &BlockRef<'c, 'c>,
    symtab: &mut ScopedSymbolTable<'c>,
    subject: Value<'c, 'c>,
    disc: Value<'c, 'c>,
    arms: &[TypedWhenArm],
    result_ty: Type<'c>,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();

    let Some((head, tail)) = arms.split_first() else {
        return Some(outer_block.const_i64(ctx.mlir, 0));
    };

    match head {
        TypedWhenArm::Else(body, _) => lower_typed_expr(ctx, *body, outer_block, symtab),
        TypedWhenArm::Cond { .. } => {
            ctx.emit_internal("Cannot mix condition arms in a pattern match (when subject is ...)");
            None
        }
        TypedWhenArm::Is { pattern, body, .. } => {
            let expected_disc: i64 = {
                let variant_name =
                    Intern::<String>::from_ref(type_surface_mangle_name(&pattern.value));
                let (_, disc_val, _) = ctx.lookup_variant(variant_name)?;
                disc_val as i64
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

                let val = lower_typed_expr(ctx, *body, &blk_ref, &mut inner_symtab)?;
                blk_ref.append_operation(scf_dialect::r#yield(&[val], loc));
            }

            let else_region = Region::new();
            {
                let blk = Block::new(&[]);
                else_region.append_block(blk);
                let blk_ref = else_region.first_block().unwrap();
                let val = lower_typed_pattern_when(
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
