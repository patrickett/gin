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
        let typed_ast = self.program;
        let joins = when_expr
            .place_joins
            .iter()
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

        let result_mlir = self.ty_to_mlir(expr_ty);

        let lowered = match when_expr.subject {
            Some(subject_id) => {
                let subject = self.lower_typed_expr(subject_id, block, symtab)?;
                if let Some(expr_ref) = typed_ast.expr(subject_id) {
                    let subject_definition = self
                        .program
                        .type_registry
                        .resolved_definition_for_type(expr_ref.ty);
                    let disc = if let Some(values) = subject_definition.union_literal_values() {
                        self.emit_discriminant_extend(block, subject, values.len())
                    } else {
                        self.sum_discriminant_i64(block, subject, expr_ref.ty)?
                    };
                    self.lower_typed_pattern_when(
                        block,
                        symtab,
                        subject,
                        disc,
                        expr_ref.ty,
                        &when_expr.arms,
                        result_mlir,
                        &joins,
                    )
                } else {
                    None
                }
            }
            None => self.lower_typed_boolean_when(block, symtab, &when_expr.arms, &joins),
        }?;
        for (index, (name, ty, _)) in joins.iter().enumerate() {
            symtab.assign(
                name,
                crate::Slot {
                    value: lowered[index + 1],
                    ty: ty.clone(),
                    is_slot: false,
                },
            );
        }
        Some(lowered[0])
    }

    fn lower_typed_boolean_when(
        &self,
        outer_block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
        arms: &[TypedWhenArm],
        joins: &[WhenJoin<'c>],
    ) -> Option<Vec<Value<'c, 'c>>> {
        let loc = self.location();

        let Some(head) = arms.first() else {
            let value = outer_block.const_i64(self.mlir, 0, loc);
            return Some(
                std::iter::once(value)
                    .chain(joins.iter().map(|(_, _, incoming)| *incoming))
                    .collect(),
            );
        };

        match head {
            TypedWhenArm::Else(body, _) => {
                let value = self.lower_typed_expr(*body, outer_block, symtab)?;
                self.when_yield_values(value, outer_block, symtab, joins)
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
                let value = self.lower_typed_expr(body, &then_block, &mut then_symbols)?;
                let yielded = self.when_yield_values(value, &then_block, &then_symbols, joins)?;
                then_block.append_operation(scf_dialect::r#yield(&yielded, loc));

                let else_region = Region::new();
                let else_block = Block::new(&[]);
                else_region.append_block(else_block);
                let else_block = else_region.first_block().unwrap();
                let yielded = self.lower_typed_boolean_when(
                    &else_block,
                    &mut symtab.clone(),
                    &arms[1..],
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
        disc: Value<'c, 'c>,
        subject_ty: &typecheck::ty::Ty,
        arms: &[TypedWhenArm],
        result_ty: Type<'c>,
        joins: &[WhenJoin<'c>],
    ) -> Option<Vec<Value<'c, 'c>>> {
        let loc = self.location();

        let Some((head, tail)) = arms.split_first() else {
            let value = outer_block.const_i64(self.mlir, 0, loc);
            return Some(
                std::iter::once(value)
                    .chain(joins.iter().map(|(_, _, incoming)| *incoming))
                    .collect(),
            );
        };

        match head {
            TypedWhenArm::Else(body, _) => {
                let value = self.lower_typed_expr(*body, outer_block, symtab)?;
                self.when_yield_values(value, outer_block, symtab, joins)
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
                    let value = self.lower_typed_expr(*body, outer_block, &mut inner_symtab)?;
                    return self.when_yield_values(value, outer_block, &inner_symtab, joins);
                }
                let expected_disc: i64 = {
                    let variant_name =
                        Intern::<String>::from_ref(pattern.value.surface_mangle_name());
                    let (_, disc_val, _) = self.lookup_variant(variant_name)?;
                    disc_val as i64
                };

                let expected_val = outer_block.const_i64(self.mlir, expected_disc, loc);
                let cond = outer_block.append_op(self.mlir.build_cmpi(
                    Predicates::EQ,
                    disc,
                    expected_val,
                    loc,
                ));

                let mut result_tys = vec![result_ty];
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

                    let val = self.lower_typed_expr(*body, &blk_ref, &mut inner_symtab)?;
                    let yielded = self.when_yield_values(val, &blk_ref, &inner_symtab, joins)?;
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
                        disc,
                        subject_ty,
                        tail,
                        result_ty,
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
        value: Value<'c, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &ScopedSymbolTable<'c>,
        joins: &[WhenJoin<'c>],
    ) -> Option<Vec<Value<'c, 'c>>> {
        let mut values = vec![value];
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
