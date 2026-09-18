use crate::prelude::*;
use internment::Intern;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(crate) fn lower_typed_condition(
        &self,
        condition: &typecheck::TypedCondition,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        match condition {
            typecheck::TypedCondition::Is { subject, pattern } => {
                let typed_ast = self.program;
                let subject_value = self.lower_typed_expr(*subject, block, symtab)?;
                let subject_ty = typed_ast.expr(*subject)?.ty;
                self.lower_pattern_condition(block, subject_value, subject_ty, &pattern.value)
            }
            typecheck::TypedCondition::Not(inner) => {
                let value = self.lower_typed_condition(inner, block, symtab)?;
                let one = block.const_int(self.mlir, self.mlir.i1(), 1, self.location());
                Some(block.append_op(self.mlir.build_binop(
                    "arith.xori",
                    value,
                    one,
                    self.mlir.i1(),
                    self.location(),
                )))
            }
            typecheck::TypedCondition::And(left, right) => {
                let left = self.lower_typed_condition(left, block, symtab)?;
                self.lower_short_circuit_condition(left, right, false, block, symtab)
            }
            typecheck::TypedCondition::Or(left, right) => {
                let left = self.lower_typed_condition(left, block, symtab)?;
                self.lower_short_circuit_condition(left, right, true, block, symtab)
            }
        }
    }

    pub(crate) fn lower_pattern_condition(
        &self,
        block: &BlockRef<'c, 'c>,
        subject_value: Value<'c, 'c>,
        subject_ty: &Ty,
        pattern: &Pattern,
    ) -> Option<Value<'c, 'c>> {
        let typed_ast = self.program;
        let binding = matches!(
            pattern,
            Pattern::Nominal(name, _)
                if name.as_str() == "_"
                    || name
                        .as_str()
                        .chars()
                        .next()
                        .is_some_and(|character| character.is_ascii_lowercase())
        );
        if binding {
            return Some(block.const_int(self.mlir, self.mlir.i1(), 1, self.location()));
        }
        if let Pattern::InRange { bounds, .. } = pattern {
            let (min, max) = match bounds {
                InRangeBounds::Literal(min, max) => (*min, *max),
                InRangeBounds::LiteralToTag(_, _) => return None,
                InRangeBounds::Tag(name) => {
                    let ty = &typed_ast.tags.get(&typecheck::TagId(*name))?.resolved_ty;
                    let hull = typed_ast
                        .type_registry
                        .integer_validity_for_type(ty)?
                        .domain()
                        .storage_hull()?;
                    (hull.min(), hull.max())
                }
            };
            let hull = typed_ast
                .type_registry
                .integer_validity_for_type(subject_ty)?
                .domain()
                .storage_hull()?;
            let min = min.max(hull.min());
            let max = max.min(hull.max());
            if min > max {
                return Some(block.const_int(self.mlir, self.mlir.i1(), 0, self.location()));
            }
            let min = ast::integer::to_i128(min)?;
            let max = ast::integer::to_i128(max)?;
            let min_value =
                block.const_int(self.mlir, subject_value.r#type(), min, self.location());
            let max_value =
                block.const_int(self.mlir, subject_value.r#type(), max, self.location());
            let signed = min.is_negative()
                || typed_ast
                    .type_registry
                    .nominal_integer_interpretation_for_type(subject_ty)
                    .or_else(|| subject_ty.anonymous_operation_interpretation())
                    == Some(ast::ty::IntegerInterpretation::Signed);
            let lower = block.append_op(self.mlir.build_cmpi(
                if signed {
                    Predicates::SGE
                } else {
                    Predicates::UGE
                },
                subject_value,
                min_value,
                self.location(),
            ));
            let upper = block.append_op(self.mlir.build_cmpi(
                if signed {
                    Predicates::SLE
                } else {
                    Predicates::ULE
                },
                subject_value,
                max_value,
                self.location(),
            ));
            return Some(block.append_op(self.mlir.build_binop(
                "arith.andi",
                lower,
                upper,
                self.mlir.i1(),
                self.location(),
            )));
        }
        if let typecheck::ty::Ty::ResultFamily { alternatives, .. } = subject_ty
            && let Pattern::Nominal(name, _) = pattern
        {
            let discriminant = alternatives
                .iter()
                .position(|alternative| alternative.label == *name)?;
            let expected = block.const_int(
                self.mlir,
                subject_value.r#type(),
                discriminant as i128,
                self.location(),
            );
            return Some(block.append_op(self.mlir.build_cmpi(
                Predicates::EQ,
                subject_value,
                expected,
                self.location(),
            )));
        }
        if let Pattern::Literal(literal, _) = pattern
            && let Some(validity) = typed_ast
                .type_registry
                .integer_validity_for_type(subject_ty)
        {
            let value = match literal {
                ast::Literal::Number(value) => (*value as u64).into(),
                ast::Literal::Int(value) => *value,
                _ => return None,
            };
            if !validity.domain().contains(value) {
                return Some(block.const_int(self.mlir, self.mlir.i1(), 0, self.location()));
            }
            let value = ast::integer::to_i128(value)?;
            let expected =
                block.const_int(self.mlir, subject_value.r#type(), value, self.location());
            return Some(block.append_op(self.mlir.build_cmpi(
                Predicates::EQ,
                subject_value,
                expected,
                self.location(),
            )));
        }
        let subject_definition = typed_ast
            .type_registry
            .resolved_definition_for_type(subject_ty);
        let discriminant = if let Some(values) = subject_definition.union_literal_values() {
            self.emit_discriminant_extend(block, subject_value, values.len())
        } else {
            self.sum_discriminant_i64(block, subject_value, subject_ty)?
        };
        let variant_name = Intern::<String>::from_ref(pattern.surface_mangle_name());
        let expected = match &subject_definition {
            Ty::Union { variants, .. } => variants
                .iter()
                .position(|variant| variant.name == variant_name)?,
            _ => self.lookup_variant(variant_name)?.1,
        };
        let expected = block.const_i64(self.mlir, expected as i64, self.location());
        Some(block.append_op(self.mlir.build_cmpi(
            Predicates::EQ,
            discriminant,
            expected,
            self.location(),
        )))
    }

    fn lower_short_circuit_condition(
        &self,
        left: Value<'c, 'c>,
        right: &typecheck::TypedCondition,
        short_circuit_value: bool,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let then_region = Region::new();
        let else_region = Region::new();
        for (region, evaluate_right) in [
            (&then_region, !short_circuit_value),
            (&else_region, short_circuit_value),
        ] {
            let region_block = Block::new(&[]);
            region.append_block(region_block);
            let region_block = region.first_block().unwrap();
            let value = if evaluate_right {
                self.lower_typed_condition(right, &region_block, &mut symtab.clone())?
            } else {
                region_block.const_int(
                    self.mlir,
                    self.mlir.i1(),
                    i128::from(short_circuit_value),
                    self.location(),
                )
            };
            region_block.append_operation(scf_dialect::r#yield(&[value], self.location()));
        }
        let operation = block.append_operation(scf_dialect::r#if(
            left,
            &[self.mlir.i1()],
            then_region,
            else_region,
            self.location(),
        ));
        Some(operation.result(0).ok()?.into())
    }

    pub(crate) fn bind_typed_condition_patterns(
        &self,
        condition: &typecheck::TypedCondition,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<()> {
        match condition {
            typecheck::TypedCondition::Is { subject, pattern } => {
                if matches!(pattern.value, Pattern::InRange { .. }) {
                    return Some(());
                }
                let typed_ast = self.program;
                let value = self.lower_typed_expr(*subject, block, symtab)?;
                let ty = typed_ast.expr(*subject)?.ty;
                self.bind_pattern_payload_fields(block, &pattern.value, value, ty, symtab);
            }
            typecheck::TypedCondition::Not(_) => {}
            typecheck::TypedCondition::And(left, right) => {
                self.bind_typed_condition_patterns(left, block, symtab)?;
                self.bind_typed_condition_patterns(right, block, symtab)?;
            }
            typecheck::TypedCondition::Or(_, _) => {}
        }
        Some(())
    }
}
