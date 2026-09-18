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
                let typed = self.program;
                let phis = loop_expr
                    .place_phis
                    .iter()
                    .filter(|version| {
                        let place = typed.place_versions[version.0 as usize].place;
                        symtab
                            .get(&typed.places[place.0 as usize].name)
                            .is_none_or(|binding| !binding.is_slot)
                    })
                    .map(|version| {
                        let place = typed.place_versions[version.0 as usize].place;
                        let place = &typed.places[place.0 as usize];
                        let binding = symtab.get(&place.name)?.clone();
                        let incoming = if binding.is_slot {
                            block.load_typed(
                                self,
                                binding.value,
                                self.ty_to_mlir(&binding.ty),
                                loc,
                            )?
                        } else {
                            binding.value
                        };
                        Some((place.name, place.ty.clone(), incoming))
                    })
                    .collect::<Option<Vec<_>>>()?;
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
                    before_symbols.assign(
                        name,
                        crate::Slot {
                            value,
                            ty: ty.clone(),
                            is_slot: false,
                        },
                    );
                    forwarded.push(value);
                }
                let condition_value =
                    self.lower_typed_condition(condition, &before_block, &mut before_symbols)?;
                before_block.append_operation(scf_dialect::condition(
                    condition_value,
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
                        crate::Slot {
                            value: after_block.argument(index).ok()?.into(),
                            ty: ty.clone(),
                            is_slot: false,
                        },
                    );
                }
                self.bind_typed_condition_patterns(condition, &after_block, &mut body_symtab)?;
                for statement in &loop_expr.stmts {
                    self.lower_typed_values(*statement, &after_block, &mut body_symtab)?;
                }
                let mut backedges = Vec::with_capacity(phis.len());
                for (name, ty, _) in &phis {
                    let binding = body_symtab.get(name)?;
                    backedges.push(if binding.is_slot {
                        after_block.load_typed(self, binding.value, self.ty_to_mlir(ty), loc)?
                    } else {
                        binding.value
                    });
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
                        crate::Slot {
                            value: operation.result(index).ok()?.into(),
                            ty: ty.clone(),
                            is_slot: false,
                        },
                    );
                }
                Some(block.const_i64(self.mlir, 0, loc))
            }
            typecheck::TypedLoopKind::ForIn {
                variable,
                iterable,
                plan,
            } => {
                let index_ty = Type::index(self.mlir);
                let typed = self.program;

                if let typecheck::IterationPlan::FixedArray { element, length } = plan {
                    let Some(length) = length
                        .as_const_int()
                        .and_then(|value| usize::try_from(value).ok())
                    else {
                        self.emit_internal("fixed-array iteration requires a constant length");
                        return None;
                    };
                    let Some(array) = self.lower_typed_expr(*iterable, block, symtab) else {
                        self.emit_internal("fixed-array iterable did not lower to a value");
                        return None;
                    };
                    let element_ty = self.ty_to_mlir(element);
                    symtab.push_scope();
                    for index in 0..length {
                        let value = block.append_op(self.mlir.llvm_extractvalue(
                            array,
                            i64::try_from(index).ok()?,
                            element_ty,
                            loc,
                        ));
                        symtab.insert(*variable, value, element.clone(), false);
                        for stmt in &loop_expr.stmts {
                            self.lower_typed_values(*stmt, block, symtab)?;
                        }
                    }
                    symtab.pop_scope();
                    return Some(block.const_i64(self.mlir, 0, loc));
                }

                let phis = loop_expr
                    .place_phis
                    .iter()
                    .filter(|version| {
                        let place = typed.place_versions[version.0 as usize].place;
                        symtab
                            .get(&typed.places[place.0 as usize].name)
                            .is_none_or(|binding| !binding.is_slot)
                    })
                    .map(|version| {
                        let place = typed.place_versions[version.0 as usize].place;
                        let place = &typed.places[place.0 as usize];
                        let binding = symtab.get(&place.name)?.clone();
                        let incoming = if binding.is_slot {
                            block.load_typed(
                                self,
                                binding.value,
                                self.ty_to_mlir(&binding.ty),
                                loc,
                            )?
                        } else {
                            binding.value
                        };
                        Some((place.name, place.ty.clone(), incoming))
                    })
                    .collect::<Option<Vec<_>>>()?;
                let phi_types = phis
                    .iter()
                    .map(|(_, ty, _)| self.ty_to_mlir(ty))
                    .collect::<Vec<_>>();
                let initial_values = phis
                    .iter()
                    .map(|(_, _, incoming)| *incoming)
                    .collect::<Vec<_>>();

                let (start_idx, end_idx) = match plan {
                    typecheck::IterationPlan::Range => {
                        let Some(expr_ref) = self.program.expr(*iterable) else {
                            self.emit_internal("for-in: iterable expression not found");
                            return None;
                        };
                        let typecheck::TypedExprKind::Range { start, end } = &expr_ref.kind else {
                            self.emit_internal("range iteration plan has a non-range source");
                            return None;
                        };
                        let mut lower_bound = |expr| {
                            let value = self.lower_typed_expr(expr, block, symtab)?;
                            let interpretation = self
                                .program
                                .type_registry
                                .integer_operation_interpretation_for_type(
                                    self.program.expr(expr)?.ty,
                                );
                            let operation =
                                if interpretation == Some(ast::ty::IntegerInterpretation::Signed) {
                                    "arith.index_cast"
                                } else {
                                    "arith.index_castui"
                                };
                            let operation = OperationBuilder::new(operation, loc)
                                .add_operands(&[value])
                                .add_results(&[index_ty])
                                .build()
                                .ok()?;
                            Some(block.append_op(operation))
                        };
                        (lower_bound(*start)?, lower_bound(*end)?)
                    }
                    typecheck::IterationPlan::FixedArray { .. } => {
                        self.emit_internal("fixed-array iteration is not lowered yet");
                        return None;
                    }
                    typecheck::IterationPlan::Unresolved => {
                        self.emit_internal("for-in source has no resolved iteration plan");
                        return None;
                    }
                };

                let step_i64 = block.const_i64(self.mlir, 1, loc);
                let step_idx = block.append_op(arith_dialect::index_cast(step_i64, index_ty, loc));

                let loop_region = Region::new();
                {
                    let mut arguments = vec![(index_ty, loc)];
                    arguments.extend(phi_types.iter().map(|ty| (*ty, loc)));
                    let loop_blk = Block::new(&arguments);
                    loop_region.append_block(loop_blk);
                    let loop_blk_ref = loop_region.first_block().unwrap();

                    let iv: Value = loop_blk_ref.argument(0).unwrap().into();
                    let iv_i64 =
                        loop_blk_ref.append_op(arith_dialect::index_cast(iv, self.mlir.i64(), loc));

                    let mut loop_symtab = symtab.clone();
                    loop_symtab.insert(*variable, iv_i64, Ty::i64(), false);
                    for (index, (name, ty, _)) in phis.iter().enumerate() {
                        loop_symtab.assign(
                            name,
                            crate::Slot {
                                value: loop_blk_ref.argument(index + 1).ok()?.into(),
                                ty: ty.clone(),
                                is_slot: false,
                            },
                        );
                    }

                    for stmt_id in &loop_expr.stmts {
                        self.lower_typed_values(*stmt_id, &loop_blk_ref, &mut loop_symtab)?;
                    }

                    let mut backedges = Vec::with_capacity(phis.len());
                    for (name, ty, _) in &phis {
                        let binding = loop_symtab.get(name)?;
                        backedges.push(if binding.is_slot {
                            loop_blk_ref.load_typed(
                                self,
                                binding.value,
                                self.ty_to_mlir(ty),
                                loc,
                            )?
                        } else {
                            binding.value
                        });
                    }
                    loop_blk_ref.append_operation(scf_dialect::r#yield(&backedges, loc));
                }

                let operation = OperationBuilder::new("scf.for", loc)
                    .add_operands(&[start_idx, end_idx, step_idx])
                    .add_operands(&initial_values)
                    .add_results(&phi_types)
                    .add_regions([loop_region])
                    .build()
                    .map_err(|error| self.emit_internal(format!("for loop lowering: {error}")))
                    .ok()?;
                let operation = block.append_operation(operation);
                for (index, (name, ty, _)) in phis.iter().enumerate() {
                    symtab.assign(
                        name,
                        crate::Slot {
                            value: operation.result(index).ok()?.into(),
                            ty: ty.clone(),
                            is_slot: false,
                        },
                    );
                }
                Some(block.const_i64(self.mlir, 0, loc))
            }
        }
    }
}
