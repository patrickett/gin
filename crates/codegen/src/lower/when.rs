use super::join::PlaceMergeKind;
use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;
use internment::Intern;
use melior::ir::operation::OperationLike;
use typecheck::TypedWhenArm;

type WhenJoin<'c> = (Intern<String>, Ty, Value<'c, 'c>);

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(crate) fn lower_typed_when_values(
        &self,
        when_expr: &typecheck::TypedWhenExpr,
        expr_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let joins =
            self.incoming_place_bindings(&when_expr.place_joins, symtab, PlaceMergeKind::Branch)?;

        let result_types = self.result_types_to_mlir(expr_ty);
        let subject_ty = when_expr
            .subject
            .and_then(|id| self.program.exprs.ty_of(id))
            .map(|ty| self.program.type_registry.resolved_definition_for_type(ty));
        if !result_types.is_empty()
            && !when_expr
                .arms
                .iter()
                .any(|arm| matches!(arm, TypedWhenArm::Else(..)))
            && !typecheck::analysis::when_is_exhaustive(
                subject_ty.as_ref(),
                &when_expr.arms,
                self.program.type_registry,
            )
        {
            // codegen-when-result-unavailable
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::WhenResultUnavailable),
                "a value-producing when must cover every case".into(),
            );
            return None;
        }

        let subject = match when_expr.subject {
            Some(id) => Some(self.lower_typed_expr(id, block, symtab)?),
            None => None,
        };
        let previous = when_expr
            .subject
            .zip(subject)
            .and_then(|(id, value)| self.when_subjects.borrow_mut().insert(id, value));
        let lowered = self.lower_typed_boolean_when(
            block,
            symtab,
            &when_expr.arms,
            &result_types,
            &joins,
            when_expr.subject,
        );
        if let Some(id) = when_expr.subject {
            self.when_subjects.borrow_mut().remove(&id);
            if let Some(previous) = previous {
                self.when_subjects.borrow_mut().insert(id, previous);
            }
        }
        let lowered = lowered?;
        for (index, (name, ty, _)) in joins.iter().enumerate() {
            symtab.assign(
                name,
                crate::Slot::value(lowered[result_types.len() + index], ty.clone()),
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
        subject: Option<typecheck::ExprId>,
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
            TypedWhenArm::Is {
                pattern,
                body,
                owned_bindings,
                arm_span,
            } => {
                let subject_id = subject?;
                let mut arms = arms.to_vec();
                arms[0] = TypedWhenArm::Cond {
                    coverage: vec![pattern.value.clone()],
                    condition: typecheck::TypedCondition::Is {
                        subject: subject_id,
                        pattern: pattern.clone(),
                    },
                    body: *body,
                    owned_bindings: owned_bindings.clone(),
                    arm_span: *arm_span,
                };
                self.lower_typed_boolean_when(
                    outer_block,
                    symtab,
                    &arms,
                    result_types,
                    joins,
                    subject,
                )
            }
            TypedWhenArm::Cond { condition, .. } => {
                let condition_value = self.lower_typed_condition(condition, outer_block, symtab)?;
                let then_region = Region::new();
                let then_block = Block::new(&[]);
                then_region.append_block(then_block);
                let then_block = then_region.first_block().unwrap();
                let mut then_symbols = symtab.clone();
                self.bind_typed_condition_patterns(
                    &condition_value,
                    &then_block,
                    &mut then_symbols,
                )?;
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
                    subject,
                )?;
                else_block.append_operation(scf_dialect::r#yield(&yielded, loc));
                let result_types = yielded.iter().map(Value::r#type).collect::<Vec<_>>();
                let operation = outer_block.append_scf_if(
                    condition_value.value,
                    &result_types,
                    then_region,
                    else_region,
                    loc,
                );
                Some(operation.results().map(Value::from).collect())
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
        if result_types.is_empty() {
            values.clear();
        }
        if typecheck::expression_returns(self.program, body) {
            values = result_types
                .iter()
                .map(|ty| block.append_op(self.mlir.llvm_undef(*ty, self.location())))
                .collect();
        }
        if values.len() != result_types.len() {
            self.emit_internal("when arm result count differs from its expression type");
            return None;
        }
        for (index, (value, target)) in values.iter_mut().zip(result_types).enumerate() {
            if typecheck::expression_returns(self.program, body) {
                break;
            }
            let source = match ty {
                Ty::Sequence(fields) => &fields.get(index)?.ty,
                _ => ty,
            };
            *value = self.convert_integer_value(*value, source, *target, block)?;
        }
        for (name, ty, _) in joins {
            let binding = symtab.get(name)?;
            values.push(self.slot_binding_value(binding, ty, block, self.location())?);
        }
        Some(values)
    }
}
