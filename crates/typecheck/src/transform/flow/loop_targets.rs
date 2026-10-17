use super::*;

pub(super) struct LoopTargets {
    pub expressions: Vec<ExprId>,
    pub rebound: HashSet<Intern<String>>,
    bindings: HashMap<PlaceId, ReferenceTargetSet>,
}

impl LoopTargets {
    pub fn new(ctx: &WalkExprCtx<'_>, roots: &[ExprId]) -> Option<Self> {
        let mut expressions = Vec::new();
        let mut pending = roots.to_vec();
        let mut seen = HashSet::new();
        while let Some(expression) = pending.pop() {
            if !seen.insert(expression) {
                continue;
            }
            let kind = ctx.typed.exprs.kind_of(expression)?;
            let replacement = match kind {
                TypedExprKind::RecordSet { value, .. }
                | TypedExprKind::AggregateSet { value, .. }
                | TypedExprKind::BufSet { value, .. } => Some(*value),
                TypedExprKind::Reassign { value, .. }
                    if !ctx.is_local_reference_binding(expression) =>
                {
                    Some(*value)
                }
                _ => None,
            };
            if let Some(value) = replacement {
                let mut contains_references = ctx
                    .typed
                    .exprs
                    .ty_of(value)
                    .is_some_and(|ty| is_reference_type(ctx.typed, ty));
                ctx.typed
                    .exprs
                    .references_of(value.index())
                    .visit(&mut |value| contains_references |= !value.sources.is_empty());
                if contains_references {
                    return None;
                }
            }
            expressions.push(expression);
            let _ = crate::typed::walk_expr_children(kind, &mut |child| {
                pending.push(child);
                std::ops::ControlFlow::Continue(())
            });
        }

        let mut bindings = HashMap::<PlaceId, ReferenceTargetSet>::new();
        let mut dependents = HashMap::<PlaceId, HashSet<PlaceId>>::new();
        let mut rebound = HashSet::new();
        for expression in &expressions {
            if !ctx.is_local_reference_binding(*expression) {
                continue;
            }
            let (name, value, reassign) = match ctx.typed.exprs.kind_of(*expression)? {
                TypedExprKind::Bind {
                    name,
                    body,
                    unassigned: false,
                    ..
                } => (*name, *body, false),
                TypedExprKind::Reassign { name, value, .. } => (*name, *value, true),
                _ => continue,
            };
            let destination = ctx.typed.exprs.place_of(*expression).flatten()?;
            let source = Self::source(ctx, value)?;
            let source_place = ctx.typed.exprs.place_of(source).flatten()?;
            bindings
                .entry(destination)
                .or_default()
                .extend(&Self::initial_targets(ctx, *expression).unwrap_or_default());
            if let Some(incoming) = ctx.flow_ctx.record_targets.get(&name) {
                bindings.entry(destination).or_default().extend(incoming);
            }
            bindings
                .entry(source_place)
                .or_default()
                .extend(&Self::initial_targets(ctx, source)?);
            dependents
                .entry(source_place)
                .or_default()
                .insert(destination);
            if reassign {
                rebound.insert(name);
            }
        }

        let mut pending: Vec<_> = bindings.keys().copied().collect();
        let mut queued: HashSet<_> = pending.iter().copied().collect();
        while let Some(source) = pending.pop() {
            queued.remove(&source);
            let targets = bindings
                .get(&source)
                .expect("loop alias worklist contains only known bindings")
                .clone();
            for destination in dependents.get(&source).into_iter().flatten() {
                let values = bindings
                    .get_mut(destination)
                    .expect("loop alias dependency has a known destination");
                let previous = values.iter().count();
                values.extend(&targets);
                if values.iter().count() != previous && queued.insert(*destination) {
                    pending.push(*destination);
                }
            }
        }
        Some(Self {
            expressions,
            rebound,
            bindings,
        })
    }

    fn initial_targets(ctx: &WalkExprCtx<'_>, expression: ExprId) -> Option<ReferenceTargetSet> {
        let mut targets = ctx.record_targets_for(expression).unwrap_or_default();
        if let Some(actual) = ctx
            .typed
            .exprs
            .target_group_of(expression)
            .and_then(Option::as_ref)
        {
            targets.extend(actual);
        }
        let known = targets.iter().next().is_some();
        known.then_some(targets)
    }

    fn source(ctx: &WalkExprCtx<'_>, mut expression: ExprId) -> Option<ExprId> {
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(expression) {
                return None;
            }
            match ctx.typed.exprs.kind_of(expression)? {
                TypedExprKind::Ref(inner) | TypedExprKind::Rematerialize { source: inner, .. } => {
                    expression = *inner
                }
                TypedExprKind::Deref(inner)
                    if is_reference_type(ctx.typed, ctx.typed.exprs.ty_of(*inner)?) =>
                {
                    expression = *inner;
                }
                TypedExprKind::Bind {
                    body,
                    stmts,
                    unassigned: false,
                    ..
                } if stmts.is_empty() => {
                    expression = *body;
                }
                TypedExprKind::FnCall { args: None, .. } | TypedExprKind::SelfRef { .. } => {
                    let place = ctx.typed.exprs.place_of(expression).flatten()?;
                    return ctx
                        .typed
                        .places
                        .get(place.0 as usize)?
                        .parent
                        .is_none()
                        .then_some(expression);
                }
                _ => return None,
            }
        }
    }

    pub fn targets_for(
        &self,
        ctx: &WalkExprCtx<'_>,
        expression: ExprId,
    ) -> Option<ReferenceTargetSet> {
        if self.bindings.is_empty() {
            return ctx.record_targets_for(expression);
        }
        let (base, fields) = Self::projection_path(ctx.typed, expression)?;
        let source = Self::source(ctx, base)?;
        let place = ctx.typed.exprs.place_of(source).flatten()?;
        let mut targets = self
            .bindings
            .get(&place)
            .cloned()
            .or_else(|| Self::initial_targets(ctx, source))?;
        for index in fields.into_iter().rev() {
            targets = targets.projected(|base| ReferenceTargetGroup::Field {
                base: Box::new(base.clone()),
                index,
            });
        }
        Some(targets)
    }

    fn projection_path(
        typed: &TypedFileAst,
        mut expression: ExprId,
    ) -> Option<(ExprId, Vec<usize>)> {
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(expression) {
                return None;
            }
            let TypedExprKind::AggregateGet { base, index } = typed.exprs.kind_of(expression)?
            else {
                return Some((expression, fields));
            };
            if is_reference_type(typed, typed.exprs.ty_of(expression)?) {
                return None;
            }
            fields.push(*index);
            expression = *base;
        }
    }
}

#[cfg(test)]
#[path = "loop_targets_tests.rs"]
mod tests;
