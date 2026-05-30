use super::lower_typed_expr;
use crate::{prelude::*, ty_to_mlir};
use ast::type_surface_mangle_name;
use internment::Intern;
use typed_ast::TypedIfCondition;
use typed_ast::ty::Ty;

/// Lower a typed-arena `if` expression.
pub(crate) fn lower_typed_if<'c>(
    ctx: &CodegenContext<'_, 'c>,
    if_expr: &typed_ast::TypedIfExpr,
    block: &BlockRef<'c, 'c>,
    symtab: &mut ScopedSymbolTable<'c>,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();
    let typed_ast = ctx.typed_ast?;

    // 1. Lower the condition
    let cond_val = match &if_expr.condition {
        TypedIfCondition::Bool(cond_id) => {
            let cond_i64 = lower_typed_expr(ctx, *cond_id, block, symtab)?;
            let op = match OperationBuilder::new("arith.trunci", loc)
                .add_operands(&[cond_i64])
                .add_results(&[ctx.mlir.i1()])
                .build()
            {
                Ok(op) => op,
                Err(e) => {
                    ctx.emit_internal(format!("arith.trunci: {e}"));
                    return None;
                }
            };
            block.append_op(op)
        }
        TypedIfCondition::Pattern { subject, pattern } => {
            let subject_val = lower_typed_expr(ctx, *subject, block, symtab)?;
            let surface_name = type_surface_mangle_name(&pattern.value);
            let variant_name = Intern::<String>::from_ref(surface_name);
            let (_, expected_disc, _) = match ctx.lookup_variant(variant_name) {
                Some(v) => v,
                None => {
                    ctx.emit_internal(format!("Unknown variant '{surface_name}' in if pattern"));
                    return None;
                }
            };
            let disc = block.append_op(ctx.mlir.llvm_extractvalue(subject_val, 0, ctx.mlir.i64()));
            let expected_val = block.const_i64(ctx.mlir, expected_disc as i64);
            block.append_op(ctx.mlir.build_cmpi(Predicates::EQ, disc, expected_val))
        }
    };

    // 2. Determine return type from typed AST
    let ret_ty = match if_expr.ret {
        Some(ret_id) => typed_ast
            .expr(ret_id)
            .map(|e| e.ty.clone())
            .unwrap_or(Ty::i64()),
        None => Ty::i64(),
    };
    let result_mlir = ty_to_mlir(
        &ret_ty,
        ctx.mlir,
        ctx.typed_ast.unwrap(),
        ctx.trait_registry,
    );

    // 3. Build then region
    let then_region = Region::new();
    {
        let blk = Block::new(&[]);
        then_region.append_block(blk);
        let blk_ref = then_region.first_block().unwrap();
        let mut inner_symtab = symtab.clone();

        if let TypedIfCondition::Pattern { subject, pattern } = &if_expr.condition {
            let subject_val = lower_typed_expr(ctx, *subject, block, symtab)?;
            super::bind_pattern_payload_fields(
                ctx,
                &blk_ref,
                &pattern.value,
                subject_val,
                &mut inner_symtab,
            );
        }

        for stmt_id in &if_expr.stmts {
            lower_typed_expr(ctx, *stmt_id, &blk_ref, &mut inner_symtab)?;
        }

        let ret_val = match if_expr.ret {
            Some(ret_id) => lower_typed_expr(ctx, ret_id, &blk_ref, &mut inner_symtab)?,
            None => blk_ref.unit_value(ctx),
        };
        blk_ref.append_operation(scf_dialect::r#yield(&[ret_val], loc));
    }

    // 4. Build else region (placeholder — shouldn't execute with early return)
    let else_region = Region::new();
    {
        let blk = Block::new(&[]);
        else_region.append_block(blk);
        let blk_ref = else_region.first_block().unwrap();
        blk_ref.append_operation(scf_dialect::r#yield(&[blk_ref.unit_value(ctx)], loc));
    }

    // 5. Emit scf.if
    let if_op = block.append_operation(scf_dialect::r#if(
        cond_val,
        &[result_mlir],
        then_region,
        else_region,
        loc,
    ));
    Some(if_op.result(0).unwrap().into())
}
