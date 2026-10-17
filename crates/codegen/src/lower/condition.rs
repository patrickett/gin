use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;
use internment::Intern;

type PatternBindings<'c> = std::collections::BTreeMap<Intern<String>, (Value<'c, 'c>, Ty)>;

pub(super) struct LoweredCondition<'p, 'c> {
    pub(super) value: Value<'c, 'c>,
    bindings: PatternBindings<'c>,
    marker: std::marker::PhantomData<&'p ()>,
}

fn pattern_binding_names(pattern: &Pattern) -> Vec<Intern<String>> {
    let mut pending = vec![pattern];
    let mut names = Vec::new();
    while let Some(pattern) = pending.pop() {
        match pattern {
            Pattern::Nominal(name, _) => names.push(*name),
            Pattern::Generic { param_spans, .. } => {
                names.extend(param_spans.iter().map(|(name, _)| *name))
            }
            Pattern::ListCons { head, tail } => {
                pending.push(&tail.value);
                pending.push(&head.value);
            }
            Pattern::Ref { inner, .. } | Pattern::Pointer(inner) => pending.push(&inner.value),
            _ => {}
        }
    }
    names.retain(|name| {
        name.as_str() != "_"
            && name
                .as_str()
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase())
    });
    names
}

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_typed_condition<'p>(
        &self,
        condition: &'p typecheck::TypedCondition,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredCondition<'p, 'c>> {
        let mut bindings = PatternBindings::new();
        let value = self.lower_condition_value(condition, block, symtab, Some(&mut bindings))?;
        Some(LoweredCondition {
            value,
            bindings,
            marker: std::marker::PhantomData,
        })
    }

    fn lower_condition_value<'p>(
        &self,
        condition: &'p typecheck::TypedCondition,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
        mut subjects: Option<&mut PatternBindings<'c>>,
    ) -> Option<Value<'c, 'c>> {
        match condition {
            typecheck::TypedCondition::Proposition { subject } => {
                self.lower_typed_expr(*subject, block, symtab)
            }
            typecheck::TypedCondition::Compare {
                left,
                relation,
                right,
            } => {
                let left_value = self.lower_typed_expr(*left, block, symtab)?;
                let right_value = self.lower_typed_expr(*right, block, symtab)?;
                self.compare_integer_values(
                    [*left, *right],
                    [left_value, right_value],
                    *relation,
                    block,
                )
            }
            typecheck::TypedCondition::Is { subject, pattern } => {
                let subject_value = self.lower_typed_expr(*subject, block, symtab)?;
                let subject_ty = self.program.expr(*subject)?.ty;
                let matched =
                    self.lower_pattern_condition(block, subject_value, subject_ty, &pattern.value)?;
                if let Some(bindings) = subjects {
                    self.capture_pattern_bindings(
                        matched,
                        &pattern.value,
                        subject_value,
                        subject_ty,
                        block,
                        symtab,
                        bindings,
                    )?;
                }
                Some(matched)
            }
            typecheck::TypedCondition::Not(inner) => {
                let value = self.lower_condition_value(inner, block, symtab, None)?;
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
                let value =
                    self.lower_condition_value(left, block, symtab, subjects.as_deref_mut())?;
                self.lower_short_circuit_condition(value, right, false, block, symtab, subjects)
            }
            typecheck::TypedCondition::Or(left, right) => {
                let value =
                    self.lower_condition_value(left, block, symtab, subjects.as_deref_mut())?;
                self.lower_short_circuit_condition(value, right, true, block, symtab, subjects)
            }
        }
    }

    pub(super) fn compare_integer_values(
        &self,
        operands: [typecheck::ExprId; 2],
        values: [Value<'c, 'c>; 2],
        relation: ast::ProofRelation,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let types = [
            self.program.expr(operands[0])?.ty,
            self.program.expr(operands[1])?.ty,
        ];
        let interpretations = types.map(|ty| {
            self.program
                .type_registry
                .integer_operation_interpretation_for_type(ty)
        });
        let signed = interpretations.contains(&Some(ast::ty::IntegerInterpretation::Signed));
        let mut width = 0;
        for (value, interpretation) in values.iter().zip(interpretations) {
            let extra = signed && interpretation == Some(ast::ty::IntegerInterpretation::Unsigned);
            width =
                width.max(IntegerType::try_from(value.r#type()).ok()?.width() + u32::from(extra));
        }
        let operand_type = IntegerType::new(self.mlir, width).into();
        let left = self.convert_integer_value(values[0], types[0], operand_type, block)?;
        let right = self.convert_integer_value(values[1], types[1], operand_type, block)?;
        let predicate = match (relation, signed) {
            (ast::ProofRelation::Equal, _) => Predicates::EQ,
            (ast::ProofRelation::NotEqual, _) => Predicates::NE,
            (ast::ProofRelation::Less, true) => Predicates::SLT,
            (ast::ProofRelation::Less, false) => Predicates::ULT,
            (ast::ProofRelation::LessOrEqual, true) => Predicates::SLE,
            (ast::ProofRelation::LessOrEqual, false) => Predicates::ULE,
            (ast::ProofRelation::Greater, true) => Predicates::SGT,
            (ast::ProofRelation::Greater, false) => Predicates::UGT,
            (ast::ProofRelation::GreaterOrEqual, true) => Predicates::SGE,
            (ast::ProofRelation::GreaterOrEqual, false) => Predicates::UGE,
        };
        Some(
            block.append_op(
                self.mlir
                    .build_cmpi(predicate, left, right, self.location()),
            ),
        )
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
        if let Some(pointee) =
            typecheck::ty::reference_pointee_for_type(subject_ty, Some(typed_ast.type_registry))
        {
            let value = block.load_typed(
                self,
                subject_value,
                self.ty_to_mlir(&pointee),
                self.location(),
            )?;
            return self.lower_pattern_condition(block, value, &pointee, pattern);
        }
        if matches!(pattern, Pattern::ListEmpty | Pattern::ListCons { .. }) {
            return self.lower_list_pattern_condition(block, subject_value, subject_ty, pattern);
        }
        if let Pattern::InRange { bounds, .. } = pattern {
            let mut excluded = Vec::new();
            let (min, max) = match bounds {
                InRangeBounds::Literal(min, max) => (*min, *max),
                InRangeBounds::LiteralToTag(_, _) => return None,
                InRangeBounds::Tag(name) => {
                    let ty = &typed_ast.tags.get(&typecheck::TagId(*name))?.resolved_ty;
                    let domain = typed_ast
                        .type_registry
                        .integer_validity_for_type(ty)?
                        .domain();
                    match domain.predicate() {
                        ast::integer::CanonicalPredicate::Hull(_) => {}
                        ast::integer::CanonicalPredicate::HullExcluding {
                            excluded: values,
                            ..
                        } => excluded.extend(values.iter().copied()),
                        _ => {
                            // codegen-integer-domain-pattern-unsupported
                            self.emit_symptom(DiagnosticCode::Codegen(CodegenCode::IntegerDomainPatternUnsupported), "integer domain pattern requires a supported complete domain predicate".into());
                            return None;
                        }
                    }
                    let hull = domain.storage_hull()?;
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
            let min = super::integer_constant_bits(min)?;
            let max = super::integer_constant_bits(max)?;
            let min_value =
                block.const_int(self.mlir, subject_value.r#type(), min, self.location());
            let max_value =
                block.const_int(self.mlir, subject_value.r#type(), max, self.location());
            let signed = typed_ast
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
            let mut matched = block.append_op(self.mlir.build_binop(
                "arith.andi",
                lower,
                upper,
                self.mlir.i1(),
                self.location(),
            ));
            for excluded in excluded {
                if !hull.contains(excluded) {
                    continue;
                }
                let excluded = block.const_int(
                    self.mlir,
                    subject_value.r#type(),
                    super::integer_constant_bits(excluded)?,
                    self.location(),
                );
                let allowed = block.append_op(self.mlir.build_cmpi(
                    Predicates::NE,
                    subject_value,
                    excluded,
                    self.location(),
                ));
                matched = block.append_op(self.mlir.build_binop(
                    "arith.andi",
                    matched,
                    allowed,
                    self.mlir.i1(),
                    self.location(),
                ));
            }
            return Some(matched);
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
                ast::Literal::Int(value) => *value,
                _ => return None,
            };
            if !validity.domain().contains(value) {
                return Some(block.const_int(self.mlir, self.mlir.i1(), 0, self.location()));
            }
            let value = super::integer_constant_bits(value)?;
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
        let discriminant = self.sum_discriminant(block, subject_value, subject_ty)?;
        let variant_name = Intern::<String>::from_ref(pattern.surface_mangle_name());
        let expected = match &subject_definition {
            Ty::Union { variants, .. } => {
                let index = variants
                    .iter()
                    .position(|variant| variant.name == variant_name)?;
                variants[index].discriminant_value(index)
            }
            _ => (self.lookup_variant(variant_name)?.1 as u64).into(),
        };
        let expected = block.const_int(
            self.mlir,
            discriminant.r#type(),
            super::integer_constant_bits(expected)?,
            self.location(),
        );
        let tag_matches = block.append_op(self.mlir.build_cmpi(
            Predicates::EQ,
            discriminant,
            expected,
            self.location(),
        ));
        self.lower_constructor_payload_condition(
            block,
            subject_value,
            subject_ty,
            pattern,
            tag_matches,
        )
    }

    fn lower_short_circuit_condition<'p>(
        &self,
        left: Value<'c, 'c>,
        right: &'p typecheck::TypedCondition,
        short_circuit_value: bool,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
        mut subjects: Option<&mut PatternBindings<'c>>,
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
                let mut symbols = symtab.clone();
                if !short_circuit_value {
                    if let Some(bindings) = subjects.as_deref() {
                        self.bind_captured_patterns(bindings, &region_block, &mut symbols)?;
                    }
                }
                self.lower_condition_value(
                    right,
                    &region_block,
                    &mut symbols,
                    subjects.as_deref_mut(),
                )?
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
        let operation = block.append_scf_if(
            left,
            &[self.mlir.i1()],
            then_region,
            else_region,
            self.location(),
        );
        Some(operation.result(0).ok()?.into())
    }

    fn capture_pattern_bindings(
        &self,
        matched: Value<'c, 'c>,
        pattern: &Pattern,
        value: Value<'c, 'c>,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &ScopedSymbolTable<'c>,
        bindings: &mut PatternBindings<'c>,
    ) -> Option<()> {
        let names = pattern_binding_names(pattern);
        if names.is_empty() {
            return Some(());
        }
        let then_region = Region::new();
        then_region.append_block(Block::new(&[]));
        let then_block = then_region.first_block()?;
        let mut symbols = symtab.clone();
        self.bind_pattern_payload_fields(&then_block, pattern, value, ty, &mut symbols)?;
        for name in names {
            let binding = symbols.get(&name)?;
            let value =
                self.slot_binding_value(binding, &binding.ty, &then_block, self.location())?;
            let slot = match bindings.get(&name) {
                Some((slot, _)) => *slot,
                None => {
                    let slot = self.allocate_stack_slot(value.r#type())?;
                    bindings.insert(name, (slot, binding.ty.clone()));
                    slot
                }
            };
            then_block.store_typed(self, slot, value, self.location())?;
        }
        then_block.append_operation(scf_dialect::r#yield(&[], self.location()));
        let else_region = Region::new();
        else_region.append_block(Block::new(&[]));
        else_region
            .first_block()?
            .append_operation(scf_dialect::r#yield(&[], self.location()));
        block.append_scf_if(matched, &[], then_region, else_region, self.location());
        Some(())
    }

    pub(super) fn bind_typed_condition_patterns(
        &self,
        lowered: &LoweredCondition<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<()> {
        self.bind_captured_patterns(&lowered.bindings, block, symtab)
    }

    fn bind_captured_patterns(
        &self,
        bindings: &PatternBindings<'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<()> {
        for (name, (slot, ty)) in bindings {
            let value = block.load_typed(self, *slot, self.ty_to_mlir(ty), self.location())?;
            symtab.insert(*name, value, ty.clone(), false);
        }
        Some(())
    }
}
