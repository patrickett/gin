use super::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::{ArrayCode, TypeCode};

fn array_source_type(typed: &TypedFileAst, mut source: ExprId) -> Ty {
    for _ in 0..typed.exprs.kind.len() {
        match typed.exprs.kind_of(source) {
            Some(TypedExprKind::FnCall {
                target,
                args: Some(_),
                substituted_ty,
            }) => {
                return substituted_ty
                    .as_ref()
                    .or_else(|| typed.fn_return_types.get(target))
                    .or_else(|| typed.defs.get(target).map(|bind| &bind.return_type))
                    .cloned()
                    .unwrap_or_else(|| typed.exprs.ty[source.index()].clone());
            }
            Some(TypedExprKind::Ref(inner) | TypedExprKind::Deref(inner)) => source = *inner,
            Some(TypedExprKind::AggregateGet { base, index }) => {
                return super::super::symbolic_values::projected_type(typed, *base, *index)
                    .unwrap_or_else(|| typed.exprs.ty[source.index()].clone());
            }
            Some(TypedExprKind::SliceGet { .. }) => return typed.exprs.ty[source.index()].clone(),
            Some(TypedExprKind::ArrayLit(items)) => {
                let Ty::Array { elem, .. } = &typed.exprs.ty[source.index()] else {
                    return typed.exprs.ty[source.index()].clone();
                };
                return Ty::Array {
                    elem: elem.clone(),
                    size: (items.len() as i128).into(),
                };
            }
            Some(TypedExprKind::FixedArrayAlloc { size, .. }) => {
                let Ty::Array { elem, .. } = &typed.exprs.ty[source.index()] else {
                    return typed.exprs.ty[source.index()].clone();
                };
                return Ty::Array {
                    elem: elem.clone(),
                    size: size.clone(),
                };
            }
            _ => {
                return typed
                    .exprs
                    .place_of(source)
                    .flatten()
                    .map(|place| typed.places[place.0 as usize].ty.clone())
                    .unwrap_or_else(|| typed.exprs.ty[source.index()].clone());
            }
        }
    }
    unreachable!("reference expressions must form an acyclic graph");
}

pub(crate) fn field_has_array_dependency(typed: &TypedFileAst, ty: &Ty, index: usize) -> bool {
    let Ty::Record { fields, .. } = typed
        .type_registry
        .resolved_definition_for_type(ty.without_reference_wrappers())
    else {
        return false;
    };
    let Some((changed, _)) = fields.get(index) else {
        return false;
    };
    fields.iter().enumerate().any(|(field_index, (_, ty))| {
        let Ty::Array { size, .. } = typed
            .type_registry
            .resolved_definition_for_type(ty.without_reference_wrappers())
        else {
            return false;
        };
        let mut variables = HashSet::new();
        super::super::scalar_mutations::variables(&size, &mut variables);
        variables.contains(changed)
            || (field_index == index
                && variables
                    .iter()
                    .any(|name| fields.iter().any(|(field, _)| field == name)))
    })
}

impl WalkExprCtx<'_> {
    pub(super) fn check_slice_borrow(
        &mut self,
        expression: ExprId,
        source: ExprId,
        start: ExprId,
        length: ExprId,
    ) {
        let span = self.typed.exprs.span[expression.index()];
        let source_type = array_source_type(self.typed, source);
        let storage = self
            .typed
            .type_registry
            .resolved_definition_for_type(source_type.without_reference_wrappers());
        let permission = self
            .typed
            .exprs
            .kind
            .iter()
            .enumerate()
            .find_map(|(index, kind)| {
                matches!(kind, TypedExprKind::Ref(inner) if *inner == expression)
                    .then(|| self.typed.exprs.ty[index].is_mutable_ref())
            });
        let error = match permission {
            None => Some((
                // type-dependent-array-borrow
                DiagnosticCode::Type(TypeCode::Array(ArrayCode::DependentBorrow)),
                "slice storage must be borrowed with ref or mut",
            )),
            Some(true) if self.typed.call_argument_is_shared(source) => Some((
                // type-dependent-array-permission
                DiagnosticCode::Type(TypeCode::Array(ArrayCode::DependentPermission)),
                "shared storage cannot supply a mutable slice",
            )),
            _ if !matches!(storage, Ty::Array { .. } | Ty::OwnedStorage { .. }) => Some((
                DiagnosticCode::Type(TypeCode::Array(ArrayCode::DependentBorrow)),
                "slicing requires array storage or an owning storage address",
            )),
            _ => None,
        };
        if let Some((code, message)) = error {
            self.typed.exprs.facts.semantic_facts.flaws[expression.index()]
                .push(Diagnostic::new(code, message).at_span_id(span, &self.typed.span_table));
        }
        self.check_slice_bounds(expression, storage, start, length);
    }

    fn check_slice_bounds(
        &mut self,
        expression: ExprId,
        storage: Ty,
        start: ExprId,
        length: ExprId,
    ) {
        let span = self.typed.exprs.span[expression.index()];
        let start = super::super::symbolic_values::normal_expr(
            self.typed,
            None,
            &self.local_const_values,
            start,
        );
        let length = super::super::symbolic_values::normal_expr(
            self.typed,
            None,
            &self.local_const_values,
            length,
        );
        if let (Some(start), Some(length)) = (start, length) {
            let mut bounds = vec![
                Predicate::Ge(start.clone(), 0.into()),
                Predicate::Ge(length.clone(), 0.into()),
            ];
            if let Ty::Array { size, .. } = storage {
                bounds.push(Predicate::Le(
                    NormalExpr::Add(Box::new(start), Box::new(length)),
                    size,
                ));
            }
            if bounds.iter().any(|bound| {
                self.flow_ctx.constraint_env.prove(bound, &HashMap::new()) == ProveResult::Disproven
            }) {
                self.typed.exprs.facts.semantic_facts.flaws[expression.index()].push(
                    // type-dependent-array-bounds
                    Diagnostic::new(
                        ArrayCode::DependentBounds,
                        "slice is outside its backing storage",
                    )
                    .at_span_id(span, &self.typed.span_table),
                );
            }
        }
    }

    pub(super) fn check_array_reference_contract(&mut self, expression: ExprId) {
        let expected = self.typed.exprs.ty[expression.index()].clone();
        if !expected.is_ref() {
            return;
        }
        if self.typed.exprs.place_of(expression).flatten().is_some()
            || matches!(
                self.typed.exprs.kind_of(expression),
                Some(TypedExprKind::Ref(_) | TypedExprKind::FnCall { args: Some(_), .. })
            )
        {
            self.check_dependent_array_argument(
                &expected,
                expression,
                &crate::subst::DepSubst::new(),
            );
        }
    }

    pub(super) fn reject_dependent_array_mutation(&mut self, expression: ExprId) {
        // type-dependent-array-mutation
        let diagnostic = Diagnostic::new(
            ArrayCode::DependentMutation,
            "cannot change an array's length and backing storage independently",
        )
        .at_span_id(
            self.typed.exprs.span[expression.index()],
            &self.typed.span_table,
        );
        self.typed.exprs.facts.semantic_facts.flaws[expression.index()].push(diagnostic);
    }

    pub(super) fn check_array_length_mutation(&mut self, expression: ExprId, name: Intern<String>) {
        let depends = self.flow_ctx.binding_types.values().any(|ty| {
            let Ty::Array { size, .. } = self
                .typed
                .type_registry
                .resolved_definition_for_type(ty.without_reference_wrappers())
            else {
                return false;
            };
            let mut variables = HashSet::new();
            super::super::scalar_mutations::variables(&size, &mut variables);
            variables.iter().any(|variable| {
                *variable == name || variable.as_str().starts_with(&format!("{name}.$field"))
            })
        });
        if depends {
            self.reject_dependent_array_mutation(expression);
        }
    }

    pub(super) fn check_dependent_array_arguments(
        &mut self,
        parameters: &[(Intern<String>, Ty)],
        arguments: &[ExprId],
        values: &[Option<NormalExpr>],
    ) {
        let mut substitution = crate::subst::DepSubst::new();
        for ((name, _), value) in parameters.iter().zip(values) {
            if let Some(value) = value {
                substitution.consts.insert(*name, value.clone());
            }
        }
        for ((_, expected), argument) in parameters.iter().zip(arguments) {
            self.check_dependent_array_argument(expected, *argument, &substitution);
        }
    }

    pub(super) fn check_dependent_array_argument(
        &mut self,
        expected: &Ty,
        argument: ExprId,
        substitution: &crate::subst::DepSubst,
    ) {
        let mutable = expected.is_mutable_ref();
        let expected = self
            .typed
            .type_registry
            .resolved_definition_for_type(expected.without_reference_wrappers());
        let Ty::Array { size, elem } = expected else {
            return;
        };
        let actual = array_source_type(self.typed, argument);
        if mutable && actual.is_ref() && !actual.is_mutable_ref() {
            // type-dependent-array-permission
            let diagnostic = Diagnostic::new(
                ArrayCode::DependentPermission,
                "shared array storage cannot supply a mutable reference",
            )
            .at_span_id(
                self.typed.exprs.span[argument.index()],
                &self.typed.span_table,
            );
            self.typed.exprs.facts.semantic_facts.flaws[argument.index()].push(diagnostic);
        }
        let actual = self
            .typed
            .type_registry
            .resolved_definition_for_type(actual.without_reference_wrappers());
        let (actual_size, elements) = match actual {
            Ty::Array { size, elem } => (Some(size), vec![*elem]),
            Ty::Sequence(fields) => (
                Some((fields.len() as i128).into()),
                fields.into_iter().map(|field| *field.ty).collect(),
            ),
            _ => (None, Vec::new()),
        };
        self.check_array_elements(&elem, elements, argument);
        let expected_size = substitution.apply_to_normal(&size);
        let proof = actual_size.map_or(ProveResult::Unknown, |actual_size| {
            self.flow_ctx.constraint_env.prove(
                &Predicate::Eq(expected_size.clone(), actual_size),
                &HashMap::new(),
            )
        });
        if proof != ProveResult::Proven {
            // type-dependent-array-length
            let diagnostic = Diagnostic::new(
                ArrayCode::DependentLength,
                format!("array reference length must match `{size}`"),
            )
            .at_span_id(
                self.typed.exprs.span[argument.index()],
                &self.typed.span_table,
            );
            self.typed.exprs.facts.semantic_facts.flaws[argument.index()].push(diagnostic);
        }
    }

    fn check_array_elements(&mut self, element: &Ty, elements: Vec<Ty>, argument: ExprId) {
        for actual in elements {
            let expected = [ast::TyArg::Type(Box::new(element.clone()))];
            let actual = [ast::TyArg::Type(Box::new(actual))];
            if crate::analysis::unify_type_args_with_registry(
                &expected,
                &actual,
                Some(&self.typed.type_registry),
            )
            .is_err()
            {
                // type-dependent-array-element
                let diagnostic = Diagnostic::new(
                    ArrayCode::DependentElement,
                    "array reference element type does not match its parameter",
                )
                .at_span_id(
                    self.typed.exprs.span[argument.index()],
                    &self.typed.span_table,
                );
                self.typed.exprs.facts.semantic_facts.flaws[argument.index()].push(diagnostic);
                break;
            }
        }
    }

    pub(super) fn check_dependent_record_arguments(
        &mut self,
        expression: ExprId,
        arguments: &[ExprId],
        names: &[Intern<String>],
        values: &[Option<NormalExpr>],
    ) {
        let Ty::Record { fields, .. } = self
            .typed
            .type_registry
            .resolved_definition_for_type(&self.typed.exprs.ty[expression.index()])
        else {
            return;
        };
        let parameters = arguments
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                let field = names
                    .get(index)
                    .and_then(|name| fields.iter().find(|(field, _)| field == name))
                    .or_else(|| fields.get(index))?;
                Some((field.0, *field.1.clone()))
            })
            .collect::<Vec<_>>();
        self.check_dependent_array_arguments(&parameters, arguments, values);
    }
}
