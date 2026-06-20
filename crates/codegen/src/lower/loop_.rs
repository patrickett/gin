use crate::prelude::*;
use typecheck::ty::Ty;

impl<'a, 'c> CodegenContext<'a, 'c> {
    /// Lower a typed-arena loop expression.
    pub(crate) fn lower_typed_loop(
        &self,
        loop_expr: &typecheck::TypedLoop,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = self.location();

        match &loop_expr.kind {
            typecheck::TypedLoopKind::While { condition } => {
                let before_region = Region::new();
                {
                    let blk = Block::new(&[]);
                    before_region.append_block(blk);
                    let blk_ref = before_region.first_block().unwrap();
                    let cond_val =
                        self.lower_typed_expr(*condition, &blk_ref, &mut symtab.clone())?;
                    blk_ref.append_operation(scf_dialect::condition(cond_val, &[], loc));
                }

                let after_region = Region::new();
                {
                    let blk = Block::new(&[]);
                    after_region.append_block(blk);
                    let blk_ref = after_region.first_block().unwrap();
                    let mut body_symtab = symtab.clone();
                    for stmt_id in &loop_expr.stmts {
                        self.lower_typed_expr(*stmt_id, &blk_ref, &mut body_symtab)?;
                    }
                    blk_ref.append_operation(scf_dialect::r#yield(&[], loc));
                }

                block.append_operation(scf_dialect::r#while(
                    &[],
                    &[],
                    before_region,
                    after_region,
                    loc,
                ));
                Some(block.const_i64(self.mlir, 0, loc))
            }
            typecheck::TypedLoopKind::ForIn { variable, iterable } => {
                let index_ty = Type::index(self.mlir);

                let (start_i64, end_i64) =
                    if let Some(expr_ref) = self.typed_ast.and_then(|ta| ta.expr(*iterable)) {
                        match &expr_ref.kind {
                            typecheck::TypedExprKind::Range { start, end } => {
                                let s = self.lower_typed_expr(*start, block, symtab)?;
                                let e = self.lower_typed_expr(*end, block, symtab)?;
                                (s, e)
                            }
                            _ => {
                                self.emit_internal(
                                "for-in loops currently only support range iterators (start...end)",
                            );
                                return None;
                            }
                        }
                    } else {
                        self.emit_internal("for-in: iterable expression not found");
                        return None;
                    };

                let step_i64 = block.const_i64(self.mlir, 1, loc);
                let start_idx =
                    block.append_op(arith_dialect::index_cast(start_i64, index_ty, loc));
                let end_idx = block.append_op(arith_dialect::index_cast(end_i64, index_ty, loc));
                let step_idx = block.append_op(arith_dialect::index_cast(step_i64, index_ty, loc));

                let loop_region = Region::new();
                {
                    let loop_blk = Block::new(&[(index_ty, loc)]);
                    loop_region.append_block(loop_blk);
                    let loop_blk_ref = loop_region.first_block().unwrap();

                    let iv: Value = loop_blk_ref.argument(0).unwrap().into();
                    let iv_i64 =
                        loop_blk_ref.append_op(arith_dialect::index_cast(iv, self.mlir.i64(), loc));

                    let mut loop_symtab = symtab.clone();
                    loop_symtab.insert(*variable, iv_i64, Ty::i64(), false);

                    for stmt_id in &loop_expr.stmts {
                        self.lower_typed_expr(*stmt_id, &loop_blk_ref, &mut loop_symtab)?;
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
                Some(block.const_i64(self.mlir, 0, loc))
            }
        }
    }
}
