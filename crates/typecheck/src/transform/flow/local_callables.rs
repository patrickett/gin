use super::*;
use crate::typed::LocalCallableParameter;
use ast::BinderOwner;
use diagnostic::code::{CallCode, TerminalCode};

#[derive(Default)]
pub(super) struct LocalOwnershipScope {
    locals: HashSet<PlaceId>,
    reads: HashSet<(ExprId, bool)>,
    pub(super) exits: Vec<FlowContext>,
}

#[derive(Clone, Default)]
pub(super) struct LocalOwnershipSummary {
    reads: Vec<(ExprId, bool)>,
    states: Vec<(PlaceId, VarState)>,
    parameters: Vec<(usize, PlaceId)>,
    exit: FlowContext,
}

fn root_place(typed: &TypedFileAst, mut place: PlaceId) -> PlaceId {
    for _ in 0..typed.places.len() {
        let Some(parent) = typed.places[place.0 as usize].parent else {
            return place;
        };
        assert!(parent.0 < place.0);
        place = parent;
    }
    unreachable!("place parents form an acyclic arena");
}

fn local_expressions(typed: &TypedFileAst, statements: &[ExprId], body: ExprId) -> HashSet<ExprId> {
    let mut pending = statements.to_vec();
    pending.push(body);
    let mut visited = HashSet::new();
    while let Some(expression) = pending.pop() {
        if !visited.insert(expression) || typed.local_callable_parameters.contains_key(&expression)
        {
            continue;
        }
        let _ =
            crate::typed::walk_expr_children(&typed.exprs.kind[expression.index()], &mut |child| {
                pending.push(child);
                std::ops::ControlFlow::Continue(())
            });
        assert!(visited.len() <= typed.exprs.kind.len());
    }
    visited
}

impl LocalOwnershipScope {
    fn new(typed: &TypedFileAst, callable: ExprId, expressions: &HashSet<ExprId>) -> Self {
        let locals = typed
            .places
            .iter()
            .enumerate()
            .filter_map(|(index, place)| {
                if place.parent.is_some() {
                    return None;
                }
                let local = match place.binder.owner {
                    BinderOwner::LocalParameter { expression, .. } => expression == callable.0,
                    BinderOwner::Expression(expression)
                    | BinderOwner::Pattern { expression, .. } => {
                        expressions.contains(&ExprId(expression))
                    }
                    _ => false,
                };
                local.then_some(PlaceId(index as u32))
            })
            .collect();
        Self {
            locals,
            ..Self::default()
        }
    }

    fn owns(&self, typed: &TypedFileAst, place: PlaceId) -> bool {
        self.locals.contains(&root_place(typed, place))
    }

    fn finish(self, typed: &TypedFileAst, incoming: &FlowContext) -> LocalOwnershipSummary {
        let mut outgoing = incoming.clone();
        outgoing.merge_branch_states(&self.exits);
        let states = (0..typed.places.len())
            .filter_map(|index| {
                let place = PlaceId(index as u32);
                if self.owns(typed, place)
                    || outgoing.place_state(place) == incoming.place_state(place)
                {
                    return None;
                }
                outgoing.place_state(place).map(|state| (place, state))
            })
            .collect();
        let mut reads: Vec<_> = self.reads.into_iter().collect();
        reads.sort_by_key(|(expression, whole)| (expression.0, *whole));
        let mut parameters: Vec<_> = self
            .locals
            .iter()
            .filter_map(|place| match typed.places[place.0 as usize].binder.owner {
                BinderOwner::LocalParameter { slot, .. } => Some((slot as usize, *place)),
                _ => None,
            })
            .collect();
        parameters.sort_by_key(|(slot, _)| *slot);
        LocalOwnershipSummary {
            reads,
            states,
            parameters,
            exit: outgoing,
        }
    }
}

impl WalkExprCtx<'_> {
    pub(super) fn in_terminal_scope(&self, place: PlaceId) -> bool {
        self.local_scope
            .as_ref()
            .is_none_or(|scope| scope.owns(self.typed, place))
    }

    pub(super) fn record_capture_read(&mut self, expression: ExprId, whole: bool) {
        let Some(scope) = &mut self.local_scope else {
            return;
        };
        let Some(place) = self.typed.exprs.place_of(expression).flatten() else {
            return;
        };
        if !scope.owns(self.typed, place) {
            scope.reads.insert((expression, whole));
        }
    }

    pub(super) fn walk_local_callable(&mut self, callable: ExprId) {
        let TypedExprKind::Bind {
            stmts,
            body,
            unassigned,
            ..
        } = self.typed.exprs.kind[callable.index()].clone()
        else {
            unreachable!("local callable metadata identifies a binding")
        };
        if unassigned {
            return;
        }
        let parameters = self.typed.local_callable_parameters[&callable].clone();
        let expressions = local_expressions(self.typed, &stmts, body);
        let scope = LocalOwnershipScope::new(self.typed, callable, &expressions);
        let recursive = self.recursive_capture_summary(&scope, &expressions);
        self.local_ownership.insert(callable, recursive);
        let snapshot = self.snapshot();
        let parent_scope = self.local_scope.replace(scope);
        let parent_owned = std::mem::take(&mut self.owned_parameters);
        let parent_receiver = self.terminal_receiver.take();
        self.initialize_local_parameters(callable, &parameters);
        self.walk_local_body(callable, &stmts, body);
        let scope = self
            .local_scope
            .take()
            .expect("local callable retains its ownership scope");
        let summary = scope.finish(self.typed, &snapshot.flow_ctx);
        self.restore(snapshot);
        self.local_scope = parent_scope;
        self.owned_parameters = parent_owned;
        self.terminal_receiver = parent_receiver;
        self.local_ownership.insert(callable, summary);
    }

    fn recursive_capture_summary(
        &self,
        scope: &LocalOwnershipScope,
        expressions: &HashSet<ExprId>,
    ) -> LocalOwnershipSummary {
        let mut summary = LocalOwnershipSummary::default();
        for expression in expressions {
            let Some(place) = self.typed.exprs.place_of(*expression).flatten() else {
                continue;
            };
            if scope.owns(self.typed, place) {
                continue;
            }
            summary.reads.push((*expression, true));
            if self
                .typed
                .borrowed_value_needs_ownership(&self.typed.places[place.0 as usize].ty)
            {
                summary.states.push((place, VarState::MaybeConsumed));
            }
        }
        summary
    }

    fn initialize_local_parameters(
        &mut self,
        callable: ExprId,
        parameters: &[LocalCallableParameter],
    ) {
        let values = parameters
            .iter()
            .map(|parameter| (parameter.name, NormalExpr::Var(parameter.name)))
            .collect();
        for (slot, parameter) in parameters.iter().enumerate() {
            self.initialize_local_parameter_facts(parameter, &values);
            let owner = BinderOwner::LocalParameter {
                expression: callable.0,
                slot: slot as u32,
            };
            let index = self
                .typed
                .places
                .iter()
                .position(|place| place.parent.is_none() && place.binder.owner == owner)
                .expect("local callable parameters have allocated places");
            let place = PlaceId(index as u32);
            if matches!(
                parameter.convention,
                ParamConvention::Own | ParamConvention::Terminal
            ) && !is_reference_type(self.typed, &parameter.ty)
            {
                self.flow_ctx.set_place_state(place, VarState::Alive);
                if parameter.convention == ParamConvention::Own {
                    self.owned_parameters.push(place);
                }
            }
        }
    }

    fn initialize_local_parameter_facts(
        &mut self,
        parameter: &LocalCallableParameter,
        values: &HashMap<Intern<String>, NormalExpr>,
    ) {
        let name = parameter.name;
        let ty = &parameter.ty;
        self.const_bindings.remove(&name);
        self.local_const_values.remove(&name);
        self.flow_ctx.constraint_env.forget(name);
        self.flow_ctx.record_targets.remove(&name);
        self.flow_ctx.scalar_bindings.remove(&name);
        if super::super::symbolic_values::scalar_type(self.typed, ty.without_reference_wrappers()) {
            self.flow_ctx.scalar_bindings.insert(name);
        }
        self.flow_ctx.binding_comparisons.remove(&name);
        if let Some(comparisons) = comparison_values::proposition_comparisons(ty, values) {
            self.flow_ctx.binding_comparisons.insert(name, comparisons);
        }
        self.flow_ctx.binding_types.insert(name, ty.clone());
        record_constraints::assume_integer_bounds(
            self.typed,
            ty,
            NormalExpr::Var(name),
            &mut self.flow_ctx.constraint_env,
        );
        record_constraints::assume_record_constraints(
            self.typed,
            self.callable_signatures,
            name,
            ty,
            &mut self.flow_ctx.constraint_env,
        );
    }

    fn walk_local_body(&mut self, callable: ExprId, statements: &[ExprId], body: ExprId) {
        for statement in statements {
            self.walk(*statement);
            self.validate_discarded_result(*statement);
            if crate::analysis::expression_returns(self.typed.resolved_program(), *statement) {
                return;
            }
        }
        self.walk(body);
        self.validate_proven_integer_cast(callable, body);
        if !crate::analysis::expression_returns(self.typed.resolved_program(), body) {
            mark_consumed_if_local_ref(self.typed, body, self.flow_ctx);
            self.validate_terminal_exit(Some(callable));
        }
    }

    pub(super) fn apply_local_call_ownership(&mut self, call: ExprId, callable: ExprId) {
        let Some(summary) = self.local_ownership.get(&callable).cloned() else {
            // type-call-before-initialization
            let diagnostic = Diagnostic::new(
                CallCode::BeforeInitialization,
                "local callable must be initialized before it is called",
            )
            .at_span_id(self.typed.exprs.span[call.index()], &self.typed.span_table);
            self.typed.exprs.facts.semantic_facts.flaws[call.index()].push(diagnostic);
            return;
        };
        self.validate_local_argument_exits(call, callable, &summary);
        for (expression, whole) in &summary.reads {
            self.record_capture_read(*expression, *whole);
            self.check_place_use_at(*expression, *whole, call);
            self.check_reference_reads(*expression);
        }
        for (place, state) in &summary.states {
            self.flow_ctx.set_place_state(*place, *state);
            if matches!(state, VarState::Consumed | VarState::MaybeConsumed) {
                self.invalidate_consumed_capture(*place, *state, &summary.reads);
            }
        }
    }

    fn validate_local_argument_exits(
        &mut self,
        call: ExprId,
        callable: ExprId,
        summary: &LocalOwnershipSummary,
    ) {
        let TypedExprKind::FnCall {
            args: Some(arguments),
            ..
        } = self.typed.exprs.kind[call.index()].clone()
        else {
            return;
        };
        for (slot, place) in &summary.parameters {
            let Some(argument) = arguments.get(*slot) else {
                continue;
            };
            let parameter = &self.typed.local_callable_parameters[&callable][*slot];
            let actual = &self.typed.exprs.ty[argument.index()];
            if parameter.ty == *actual
                || !matches!(
                    parameter.convention,
                    ParamConvention::Own | ParamConvention::Terminal
                )
            {
                continue;
            }
            if self.pending_terminal_obligation_in(actual, Some(*place), &summary.exit) {
                // type-terminal-value-not-consumed
                let diagnostic = Diagnostic::new(
                    TerminalCode::ValueNotConsumed,
                    format!(
                        "local parameter `{}` does not consume its owned argument",
                        parameter.name
                    ),
                )
                .at_span_id(self.typed.exprs.span[call.index()], &self.typed.span_table);
                self.typed.exprs.facts.semantic_facts.flaws[call.index()].push(diagnostic);
            }
        }
    }

    fn invalidate_consumed_capture(
        &mut self,
        place: PlaceId,
        state: VarState,
        reads: &[(ExprId, bool)],
    ) {
        let incoming = self.ref_tracker.clone();
        for (expression, _) in reads {
            if self.typed.exprs.place_of(*expression).flatten() == Some(place)
                && let Some(targets) = self
                    .typed
                    .exprs
                    .target_group_of(*expression)
                    .cloned()
                    .flatten()
            {
                self.ref_tracker
                    .permanently_invalidate(&targets, self.flow_ctx);
            }
        }
        if state == VarState::MaybeConsumed {
            self.ref_tracker
                .merge_branch_validity(&[incoming, self.ref_tracker.clone()]);
        }
    }
}
