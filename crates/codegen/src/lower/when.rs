use crate::prelude::*;
use internment::Intern;
use melior::ir::operation::OperationLike;
use typecheck::TypedWhenArm;

type WhenJoin<'c> = (Intern<String>, Ty, Value<'c, 'c>);

impl<'a, 'c> CodegenContext<'a, 'c> {
    /// Lower a typed-arena `when` expression (runtime path).
    pub(crate) fn lower_typed_when(
        &self,
        when_expr: &typecheck::TypedWhenExpr,
        expr_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let values = self.lower_typed_when_values(when_expr, expr_ty, block, symtab)?;
        if self.is_empty_product(expr_ty) {
            Some(block.unit_value(self, self.location()))
        } else {
            values.single()
        }
    }

    pub(crate) fn lower_typed_when_values(
        &self,
        when_expr: &typecheck::TypedWhenExpr,
        expr_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let typed_ast = self.program;
        let joins = when_expr
            .place_joins
            .iter()
            .filter(|version| {
                let place = typed_ast.place_versions[version.0 as usize].place;
                symtab
                    .get(&typed_ast.places[place.0 as usize].name)
                    .is_none_or(|binding| !binding.is_slot)
            })
            .map(|version| {
                let place = typed_ast.place_versions[version.0 as usize].place;
                let place = &typed_ast.places[place.0 as usize];
                let binding = symtab.get(&place.name)?.clone();
                let incoming = if binding.is_slot {
                    block.load_typed(
                        self,
                        binding.value,
                        self.ty_to_mlir(&binding.ty),
                        self.location(),
                    )?
                } else {
                    binding.value
                };
                Some((place.name, place.ty.clone(), incoming))
            })
            .collect::<Option<Vec<_>>>()?;

        let result_types = self.result_types_to_mlir(expr_ty);

        let lowered = match when_expr.subject {
            Some(subject_id) => {
                let subject = self.lower_typed_expr(subject_id, block, symtab)?;
                if let Some(expr_ref) = typed_ast.expr(subject_id) {
                    self.lower_typed_pattern_when(
                        block,
                        symtab,
                        subject,
                        expr_ref.ty,
                        &when_expr.arms,
                        &result_types,
                        &joins,
                    )
                } else {
                    None
                }
            }
            None => {
                self.lower_typed_boolean_when(block, symtab, &when_expr.arms, &result_types, &joins)
            }
        }?;
        for (index, (name, ty, _)) in joins.iter().enumerate() {
            symtab.assign(
                name,
                crate::Slot {
                    value: lowered[result_types.len() + index],
                    ty: ty.clone(),
                    is_slot: false,
                },
            );
        }
        Some(LoweredValue::new(
            lowered.into_iter().take(result_types.len()).collect(),
        ))
    }

    fn lower_typed_boolean_when(
        &self,
        outer_block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
        arms: &[TypedWhenArm],
        result_types: &[Type<'c>],
        joins: &[WhenJoin<'c>],
    ) -> Option<Vec<Value<'c, 'c>>> {
        let loc = self.location();

        let Some(head) = arms.first() else {
            let mut values = result_types
                .iter()
                .map(|ty| outer_block.append_op(self.mlir.llvm_undef(*ty, loc)))
                .collect::<Vec<_>>();
            values.extend(joins.iter().map(|(_, _, incoming)| *incoming));
            return Some(values);
        };

        match head {
            TypedWhenArm::Else(body, _) => {
                let value = self.lower_typed_values(*body, outer_block, symtab)?;
                self.when_yield_values(value, *body, result_types, outer_block, symtab, joins)
            }
            TypedWhenArm::Is { .. } => {
                self.emit_internal("Is arm in boolean when — use 'when subject is ...' form");
                None
            }
            TypedWhenArm::Cond { condition, .. } => {
                let condition_value = self.lower_typed_condition(condition, outer_block, symtab)?;
                let then_region = Region::new();
                let then_block = Block::new(&[]);
                then_region.append_block(then_block);
                let then_block = then_region.first_block().unwrap();
                let mut then_symbols = symtab.clone();
                self.bind_typed_condition_patterns(condition, &then_block, &mut then_symbols)?;
                let body = match head {
                    TypedWhenArm::Cond { body, .. } => *body,
                    _ => unreachable!(),
                };
                let value = self.lower_typed_values(body, &then_block, &mut then_symbols)?;
                let yielded = self.when_yield_values(
                    value,
                    body,
                    result_types,
                    &then_block,
                    &then_symbols,
                    joins,
                )?;
                then_block.append_operation(scf_dialect::r#yield(&yielded, loc));

                let else_region = Region::new();
                let else_block = Block::new(&[]);
                else_region.append_block(else_block);
                let else_block = else_region.first_block().unwrap();
                let yielded = self.lower_typed_boolean_when(
                    &else_block,
                    &mut symtab.clone(),
                    &arms[1..],
                    result_types,
                    joins,
                )?;
                else_block.append_operation(scf_dialect::r#yield(&yielded, loc));
                let result_types = yielded.iter().map(Value::r#type).collect::<Vec<_>>();
                let operation = outer_block.append_operation(scf_dialect::r#if(
                    condition_value,
                    &result_types,
                    then_region,
                    else_region,
                    loc,
                ));
                Some(operation.results().map(Value::from).collect())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_typed_pattern_when(
        &self,
        outer_block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
        subject: Value<'c, 'c>,
        subject_ty: &typecheck::ty::Ty,
        arms: &[TypedWhenArm],
        result_types: &[Type<'c>],
        joins: &[WhenJoin<'c>],
    ) -> Option<Vec<Value<'c, 'c>>> {
        let loc = self.location();

        let Some((head, tail)) = arms.split_first() else {
            let mut values = result_types
                .iter()
                .map(|ty| outer_block.append_op(self.mlir.llvm_undef(*ty, loc)))
                .collect::<Vec<_>>();
            values.extend(joins.iter().map(|(_, _, incoming)| *incoming));
            return Some(values);
        };

        match head {
            TypedWhenArm::Else(body, _) => {
                let value = self.lower_typed_values(*body, outer_block, symtab)?;
                self.when_yield_values(value, *body, result_types, outer_block, symtab, joins)
            }
            TypedWhenArm::Cond { .. } => {
                self.emit_internal(
                    "Cannot mix condition arms in a pattern match (when subject is ...)",
                );
                None
            }
            TypedWhenArm::Is { pattern, body, .. } => {
                if matches!(
                    &pattern.value,
                    Pattern::Nominal(name, _)
                        if name.as_str() == "_"
                            || name
                                .as_str()
                                .chars()
                                .next()
                                .is_some_and(|character| character.is_ascii_lowercase())
                ) {
                    let mut inner_symtab = symtab.clone();
                    self.bind_pattern_payload_fields(
                        outer_block,
                        &pattern.value,
                        subject,
                        subject_ty,
                        &mut inner_symtab,
                    );
                    let value = self.lower_typed_values(*body, outer_block, &mut inner_symtab)?;
                    return self.when_yield_values(
                        value,
                        *body,
                        result_types,
                        outer_block,
                        &inner_symtab,
                        joins,
                    );
                }
                let cond =
                    self.lower_pattern_condition(outer_block, subject, subject_ty, &pattern.value)?;

                let mut result_tys = result_types.to_vec();
                result_tys.extend(joins.iter().map(|(_, ty, _)| self.ty_to_mlir(ty)));

                let then_region = Region::new();
                {
                    let blk = Block::new(&[]);
                    then_region.append_block(blk);
                    let blk_ref = then_region.first_block().unwrap();
                    let mut inner_symtab = symtab.clone();

                    self.bind_pattern_payload_fields(
                        &blk_ref,
                        &pattern.value,
                        subject,
                        subject_ty,
                        &mut inner_symtab,
                    );

                    let val = self.lower_typed_values(*body, &blk_ref, &mut inner_symtab)?;
                    let yielded = self.when_yield_values(
                        val,
                        *body,
                        result_types,
                        &blk_ref,
                        &inner_symtab,
                        joins,
                    )?;
                    blk_ref.append_operation(scf_dialect::r#yield(&yielded, loc));
                }

                let else_region = Region::new();
                {
                    let blk = Block::new(&[]);
                    else_region.append_block(blk);
                    let blk_ref = else_region.first_block().unwrap();
                    let val = self.lower_typed_pattern_when(
                        &blk_ref,
                        &mut symtab.clone(),
                        subject,
                        subject_ty,
                        tail,
                        result_types,
                        joins,
                    )?;
                    blk_ref.append_operation(scf_dialect::r#yield(&val, loc));
                }

                let if_op = scf_dialect::r#if(cond, &result_tys, then_region, else_region, loc);
                let result_op = outer_block.append_operation(if_op);
                Some(result_op.results().map(Value::from).collect())
            }
        }
    }

    fn when_yield_values(
        &self,
        value: LoweredValue<'c>,
        body: typecheck::ExprId,
        result_types: &[Type<'c>],
        block: &BlockRef<'c, 'c>,
        symtab: &ScopedSymbolTable<'c>,
        joins: &[WhenJoin<'c>],
    ) -> Option<Vec<Value<'c, 'c>>> {
        let mut values = value.into_values();
        let ty = self.program.expr(body)?.ty;
        if values.len() != result_types.len() {
            self.emit_internal("when arm result count differs from its expression type");
            return None;
        }
        for (index, (value, target)) in values.iter_mut().zip(result_types).enumerate() {
            let source = match ty {
                Ty::ResultRow(fields) => &fields.get(index)?.ty,
                _ => ty,
            };
            *value = self.convert_integer_value(*value, source, *target, block)?;
        }
        for (name, ty, _) in joins {
            let binding = symtab.get(name)?;
            values.push(if binding.is_slot {
                block.load_typed(self, binding.value, self.ty_to_mlir(ty), self.location())?
            } else {
                binding.value
            });
        }
        Some(values)
    }
}
