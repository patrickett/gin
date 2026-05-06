use crate::prelude::*;

use ast::for_loop_pattern_names;

impl<'c> Lower<'c> for ForInLoop {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = ctx.location();
        let index_ty = Type::index(ctx.mlir);

        // Currently only range iterators (start...end) are supported.
        let (start_expr, end_expr) = match &self.iter.0 {
            Expr::Range(range) => (&range.start, &range.end),
            _ => {
                ctx.emit_internal(
                    "for-in loops currently only support range iterators (start...end)",
                );
                return None;
            }
        };

        // Lower bounds to i64, then cast to index for scf.for.
        let start_i64 = start_expr.lower(ctx, block, symtab)?;
        let end_i64 = end_expr.lower(ctx, block, symtab)?;
        let step_i64 = block.const_i64(ctx.mlir, 1);

        let start_idx = block.append_op(arith_dialect::index_cast(start_i64, index_ty, loc));
        let end_idx = block.append_op(arith_dialect::index_cast(end_i64, index_ty, loc));
        let step_idx = block.append_op(arith_dialect::index_cast(step_i64, index_ty, loc));

        // Build the loop body region.
        // scf.for provides an index-typed induction variable as the block argument.
        let loop_region = Region::new();
        {
            let loop_blk = Block::new(&[(index_ty, loc)]);
            loop_region.append_block(loop_blk);
            let loop_blk_ref = loop_region.first_block().unwrap();

            // Cast induction variable from index to i64 and bind to the loop pattern.
            let iv: Value = loop_blk_ref.argument(0).unwrap().into();
            let iv_i64 = loop_blk_ref.append_op(arith_dialect::index_cast(iv, ctx.mlir.i64(), loc));

            let mut loop_symtab = symtab.clone();
            match for_loop_pattern_names(&self.pat.0).as_deref() {
                Some([name]) => {
                    loop_symtab.insert(name.as_str().to_string(), iv_i64);
                }
                Some([]) => {
                    ctx.emit_internal("Empty for-loop pattern is not supported");
                    return None;
                }
                Some(_) => {
                    ctx.emit_internal("Tuple patterns in for loops are not yet supported");
                    return None;
                }
                None => {
                    ctx.emit_internal("Invalid for-loop pattern (expected identifier(s))");
                    return None;
                }
            }

            for expr in &self.exprs {
                expr.lower(ctx, &loop_blk_ref, &mut loop_symtab)?;
            }

            loop_blk_ref.append_operation(scf_dialect::r#yield(&[], loc));
        }

        block.append_operation(scf_dialect::r#for(
            start_idx,
            end_idx,
            step_idx,
            loop_region,
            loc,
        ));

        Some(block.const_i64(ctx.mlir, 0))
    }
}
