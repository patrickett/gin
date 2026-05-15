use crate::{prelude::*, ty_to_mlir};
use ast::TyInfer;
use ast::ty::Ty;
use ast::type_surface_mangle_name;
use internment::Intern;

impl<'c> Lower<'c> for IfExpr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = ctx.location();

        let cond_val = match &self.condition {
            IfCondition::Bool(expr) => {
                let cond_i64 = expr.lower(ctx, block, symtab)?;
                let loc = ctx.location();
                // Truncate i64 to i1 (boolean) for scf.if condition
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
            IfCondition::Pattern { subject, pattern } => {
                let subject_val = subject.lower(ctx, block, symtab)?;
                let surface_name = type_surface_mangle_name(&pattern.value);
                let variant_name = Intern::<String>::from_ref(surface_name);
                let (_, expected_disc, _) = match ctx.lookup_variant(variant_name) {
                    Some(v) => v,
                    None => {
                        ctx.emit_internal(format!(
                            "Unknown variant '{}' in if pattern",
                            surface_name
                        ));
                        return None;
                    }
                };
                let disc =
                    block.append_op(ctx.mlir.llvm_extractvalue(subject_val, 0, ctx.mlir.i64()));
                let expected_val = block.const_i64(ctx.mlir, expected_disc as i64);
                block.append_op(ctx.mlir.build_cmpi(Predicates::EQ, disc, expected_val))
            }
        };

        let ret_ty = self
            .ret
            .value
            .as_ref()
            .map(|e| e.infer_ty(&ctx.infer_env(&std::collections::HashMap::new())))
            .unwrap_or(Ty::i64());
        let result_mlir = ty_to_mlir(&ret_ty, ctx.mlir);

        let then_region = Region::new();
        {
            let blk = Block::new(&[]);
            then_region.append_block(blk);
            let blk_ref = then_region.first_block().unwrap();
            let mut inner_symtab = symtab.clone();

            if let IfCondition::Pattern { subject, pattern } = &self.condition {
                let subject_val = subject.lower(ctx, block, symtab)?;
                super::bind_pattern_payload_fields(
                    ctx,
                    &blk_ref,
                    &pattern.value,
                    subject_val,
                    &mut inner_symtab,
                );
            }

            // Execute body expressions
            for expr in &self.body {
                expr.lower(ctx, &blk_ref, &mut inner_symtab)?;
            }

            // Yield the return value instead of calling llvm.return
            let ret_val = match &self.ret.value {
                Some(expr) => expr.lower(ctx, &blk_ref, &mut inner_symtab)?,
                None => blk_ref.unit_value(ctx),
            };
            blk_ref.append_operation(scf_dialect::r#yield(&[ret_val], loc));
        }

        // Else region: yield a dummy value (won't be used if we return)
        let else_region = Region::new();
        {
            let blk = Block::new(&[]);
            else_region.append_block(blk);
            let blk_ref = else_region.first_block().unwrap();
            // Else block should never execute when we have early return semantics
            // Yield unit as a placeholder
            blk_ref.append_operation(scf_dialect::r#yield(&[blk_ref.unit_value(ctx)], loc));
        }

        // Emit scf.if that produces the return value
        let if_op = block.append_operation(scf_dialect::r#if(
            cond_val,
            &[result_mlir],
            then_region,
            else_region,
            loc,
        ));

        // Return the produced value from the if expression
        Some(if_op.result(0).unwrap().into())
    }
}
