use crate::{prelude::*, ty_to_mlir};
use internment::Intern;
use typeck::{Ty, TyInfer};

impl<'c> Lower<'c> for IfExpr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
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
            IfCondition::Pattern { subject, tag } => {
                let subject_val = subject.lower(ctx, block, symtab)?;
                let variant_name = Intern::<String>::from_ref(tag.name());
                let (_, expected_disc, _) = match ctx.ty_env.lookup_variant(variant_name) {
                    Some(v) => v,
                    None => {
                        ctx.emit_internal(format!(
                            "Unknown variant '{}' in if pattern",
                            tag.name()
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

        // Infer the return type from the if block's return expression
        let ret_ty = self
            .ret
            .0
            .as_ref()
            .map(|e| e.infer_ty(&ctx.ty_env.infer_env(&std::collections::HashMap::new())))
            .unwrap_or(Ty::Int {
                width: 64,
                signed: true,
                value: None,
            });
        let result_mlir = ty_to_mlir(&ret_ty, ctx.mlir);

        let then_region = Region::new();
        {
            let blk = Block::new(&[]);
            then_region.append_block(blk);
            let blk_ref = then_region.first_block().unwrap();
            let mut inner_symtab = symtab.clone();

            // Bind pattern variables if this is a pattern condition.
            if let IfCondition::Pattern { subject, tag } = &self.condition
                && let Tag::Generic(_, params, _) = tag
            {
                let subject_val = subject.lower(ctx, block, symtab)?;
                let variant_name = Intern::<String>::from_ref(tag.name());
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
                        subject_val,
                        (slot + 1) as i64,
                        field_mlir_ty,
                    ));
                    inner_symtab.insert(param_name.as_str().to_string(), extracted);
                }
            }

            // Execute body expressions
            for expr in &self.body {
                expr.lower(ctx, &blk_ref, &mut inner_symtab)?;
            }

            // Yield the return value instead of calling llvm.return
            let ret_val = match &self.ret.0 {
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
