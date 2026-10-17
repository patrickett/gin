use super::join::PlaceMergeKind;
use crate::prelude::*;
use typecheck::ty::Ty;

/// The induction variable and bounds of a `for` loop over a counted sequence.
struct CountedRange<'c> {
    variable: internment::Intern<String>,
    item_ty: Ty,
    start: Value<'c, 'c>,
    end: Value<'c, 'c>,
    /// Whether the range may iterate downward when `start > end`.
    bidirectional: bool,
}

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
            typecheck::TypedLoopKind::While {
                condition, stmts, ..
            } => {
                let phis = self.incoming_place_bindings(
                    &loop_expr.place_phis,
                    symtab,
                    PlaceMergeKind::Loop,
                )?;
                let phi_types = phis
                    .iter()
                    .map(|(_, ty, _)| self.ty_to_mlir(ty))
                    .collect::<Vec<_>>();
                let initial_values = phis
                    .iter()
                    .map(|(_, _, incoming)| *incoming)
                    .collect::<Vec<_>>();
                let before_region = Region::new();
                let before_block =
                    Block::new(&phi_types.iter().map(|ty| (*ty, loc)).collect::<Vec<_>>());
                before_region.append_block(before_block);
                let before_block = before_region.first_block().unwrap();
                let mut before_symbols = symtab.clone();
                let mut forwarded = Vec::with_capacity(phis.len());
                for (index, (name, ty, _)) in phis.iter().enumerate() {
                    let value = before_block.argument(index).ok()?.into();
                    before_symbols.assign(name, crate::Slot::value(value, ty.clone()));
                    forwarded.push(value);
                }
                let condition_value =
                    self.lower_loop_condition(condition, &before_block, &mut before_symbols)?;
                before_block.append_operation(scf_dialect::condition(
                    condition_value.value,
                    &forwarded,
                    loc,
                ));

                let after_region = Region::new();
                let after_block =
                    Block::new(&phi_types.iter().map(|ty| (*ty, loc)).collect::<Vec<_>>());
                after_region.append_block(after_block);
                let after_block = after_region.first_block().unwrap();
                let mut body_symtab = symtab.clone();
                for (index, (name, ty, _)) in phis.iter().enumerate() {
                    body_symtab.assign(
                        name,
                        crate::Slot::value(after_block.argument(index).ok()?.into(), ty.clone()),
                    );
                }
                self.bind_typed_condition_patterns(
                    &condition_value,
                    &after_block,
                    &mut body_symtab,
                )?;
                self.lower_loop_statements(stmts, &after_block, &mut body_symtab)?;
                let mut backedges = Vec::with_capacity(phis.len());
                for (name, ty, _) in &phis {
                    let binding = body_symtab.get(name)?;
                    backedges.push(self.slot_binding_value(binding, ty, &after_block, loc)?);
                }
                after_block.append_operation(scf_dialect::r#yield(&backedges, loc));

                let operation = block.append_operation(scf_dialect::r#while(
                    &initial_values,
                    &phi_types,
                    before_region,
                    after_region,
                    loc,
                ));
                for (index, (name, ty, _)) in phis.iter().enumerate() {
                    symtab.assign(
                        name,
                        crate::Slot::value(operation.result(index).ok()?.into(), ty.clone()),
                    );
                }
                Some(block.const_i64(self.mlir, 0, loc))
            }
            typecheck::TypedLoopKind::ForIn {
                iterable,
                traversal,
            } => {
                let (variable, start, end, stmts) = match traversal {
                    typecheck::SequenceTraversal::Fixed { iterations } => {
                        symtab.push_scope();
                        self.lower_typed_expr(*iterable, block, symtab)?;
                        for statements in iterations {
                            symtab.push_scope();
                            self.lower_loop_statements(statements, block, symtab)?;
                            symtab.pop_scope();
                        }
                        symtab.pop_scope();
                        return Some(block.const_i64(self.mlir, 0, loc));
                    }
                    typecheck::SequenceTraversal::RuntimeView {
                        index,
                        length,
                        stmts,
                    } => {
                        symtab.push_scope();
                        self.lower_typed_expr(*iterable, block, symtab)?;
                        let end = self.lower_typed_expr(*length, block, symtab)?;
                        let start = block.const_int(self.mlir, end.r#type(), 0, loc);
                        let index_ty = Ty::AnonymousInteger {
                            validity: self
                                .program
                                .type_registry
                                .integer_validity_for_type(self.program.expr(*length)?.ty)?
                                .clone(),
                        };
                        let result = self.lower_counted_sequence_loop(
                            loop_expr,
                            CountedRange {
                                variable: *index,
                                item_ty: index_ty,
                                start,
                                end,
                                bidirectional: false,
                            },
                            stmts,
                            block,
                            symtab,
                        );
                        symtab.pop_scope();
                        return result;
                    }
                    typecheck::SequenceTraversal::IntegerRange {
                        variable,
                        start,
                        end,
                        stmts,
                    } => (variable, start, end, stmts),
                    typecheck::SequenceTraversal::Unresolved => {
                        self.emit_internal("for source has no resolved sequence traversal");
                        return None;
                    }
                };
                let item_ty = self.program.expr(*stmts.first()?)?.ty.clone();
                let cursor_ty = self.ty_to_mlir(&item_ty);
                let start_value = self.lower_typed_expr(*start, block, symtab)?;
                let start_value = self.convert_integer_value(
                    start_value,
                    self.program.expr(*start)?.ty,
                    cursor_ty,
                    block,
                )?;
                let end_value = self.lower_typed_expr(*end, block, symtab)?;
                let end_value = self.convert_integer_value(
                    end_value,
                    self.program.expr(*end)?.ty,
                    cursor_ty,
                    block,
                )?;
                self.lower_counted_sequence_loop(
                    loop_expr,
                    CountedRange {
                        variable: *variable,
                        item_ty,
                        start: start_value,
                        end: end_value,
                        bidirectional: true,
                    },
                    stmts,
                    block,
                    symtab,
                )
            }
        }
    }

    fn lower_counted_sequence_loop(
        &self,
        loop_expr: &typecheck::TypedLoop,
        range: CountedRange<'c>,
        stmts: &[typecheck::ExprId],
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let CountedRange {
            variable,
            item_ty,
            start,
            end,
            bidirectional,
        } = range;
        let loc = self.location();
        let cursor_ty = start.r#type();
        let signed = self
            .program
            .type_registry
            .integer_operation_interpretation_for_type(&item_ty)
            == Some(ast::ty::IntegerInterpretation::Signed);
        let predicate = if signed {
            Predicates::SLT
        } else {
            Predicates::ULT
        };
        let one = block.const_int(self.mlir, cursor_ty, 1, loc);
        let step = if bidirectional {
            let descending = block.append_op(self.mlir.build_cmpi(
                if signed {
                    Predicates::SGT
                } else {
                    Predicates::UGT
                },
                start,
                end,
                loc,
            ));
            let negative_one = block.const_int(self.mlir, cursor_ty, -1, loc);
            block.append_op(arith_dialect::select(descending, negative_one, one, loc))
        } else {
            one
        };
        let phis =
            self.incoming_place_bindings(&loop_expr.place_phis, symtab, PlaceMergeKind::Loop)?;
        let active = if bidirectional {
            block.const_int(self.mlir, self.mlir.i1(), 1, loc)
        } else {
            block.append_op(self.mlir.build_cmpi(predicate, start, end, loc))
        };
        let mut types = vec![cursor_ty, self.mlir.i1()];
        types.extend(phis.iter().map(|(_, ty, _)| self.ty_to_mlir(ty)));
        let mut initial = vec![start, active];
        initial.extend(phis.iter().map(|(_, _, value)| *value));
        let before_region = Region::new();
        before_region.append_block(Block::new(
            &types.iter().map(|ty| (*ty, loc)).collect::<Vec<_>>(),
        ));
        let before = before_region.first_block()?;
        let mut condition = before.argument(1).ok()?.into();
        if let Some(storage) = self.return_storage.borrow().as_ref() {
            let returned = before.load_typed(self, storage.flag, self.mlir.i1(), loc)?;
            let zero = before.const_int(self.mlir, self.mlir.i1(), 0, loc);
            let running =
                before.append_op(self.mlir.build_cmpi(Predicates::EQ, returned, zero, loc));
            condition = before.append_op(arith_dialect::andi(condition, running, loc));
        }
        let forwarded = (0..types.len())
            .map(|index| before.argument(index).ok().map(Value::from))
            .collect::<Option<Vec<_>>>()?;
        before.append_operation(scf_dialect::condition(condition, &forwarded, loc));
        let after_region = Region::new();
        after_region.append_block(Block::new(
            &types.iter().map(|ty| (*ty, loc)).collect::<Vec<_>>(),
        ));
        let after = after_region.first_block()?;
        let cursor: Value = after.argument(0).ok()?.into();
        let mut symbols = symtab.clone();
        symbols.push_scope();
        symbols.insert(variable, cursor, item_ty.clone(), false);
        for (index, (name, ty, _)) in phis.iter().enumerate() {
            symbols.assign(
                name,
                crate::Slot::value(after.argument(index + 2).ok()?.into(), ty.clone()),
            );
        }
        self.lower_loop_statements(stmts, &after, &mut symbols)?;
        let (next, active) = if bidirectional {
            let more = after.append_op(self.mlir.build_cmpi(Predicates::NE, cursor, end, loc));
            let zero = after.const_int(self.mlir, cursor_ty, 0, loc);
            let increment = after.append_op(arith_dialect::select(more, step, zero, loc));
            let next = after.append_op(arith_dialect::addi(cursor, increment, loc));
            (next, more)
        } else {
            let next = after.append_op(arith_dialect::addi(cursor, step, loc));
            let active = after.append_op(self.mlir.build_cmpi(predicate, next, end, loc));
            (next, active)
        };
        let mut backedges = vec![next, active];
        for (name, ty, _) in &phis {
            let binding = symbols.get(name)?;
            backedges.push(self.slot_binding_value(binding, ty, &after, loc)?);
        }
        after.append_operation(scf_dialect::r#yield(&backedges, loc));
        let operation = block.append_operation(scf_dialect::r#while(
            &initial,
            &types,
            before_region,
            after_region,
            loc,
        ));
        for (index, (name, ty, _)) in phis.iter().enumerate() {
            symtab.assign(
                name,
                crate::Slot::value(operation.result(index + 2).ok()?.into(), ty.clone()),
            );
        }
        Some(block.const_i64(self.mlir, 0, loc))
    }
}
