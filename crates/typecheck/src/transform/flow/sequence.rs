use super::*;
use crate::typed::{ReferenceSource, ReferenceValue};
use diagnostic::code::SequenceCode;

impl WalkExprCtx<'_> {
    fn sequence_reference_targets(&self, value: &ReferenceValue) -> ReferenceTargetSet {
        let mut targets = value.targets();
        for source in &value.sources {
            if let ReferenceSource::Argument(argument) = source
                && let Some(group) = self
                    .definition
                    .and_then(|id| self.typed.defs.get(&id))
                    .and_then(|bind| bind.param_groups.get(argument.parameter))
                    .copied()
                    .flatten()
            {
                let mut target = ReferenceTargetGroup::Param(group);
                for index in &argument.fields {
                    target = ReferenceTargetGroup::Field {
                        base: Box::new(target),
                        index: *index,
                    };
                }
                targets.extend(&ReferenceTargetSet::singleton(target));
            }
        }
        targets
    }

    fn sequence_access(&self, expression: ExprId) -> (ReferenceTargetSet, HashSet<ReferenceLoan>) {
        let value = self.typed.exprs.references_of(expression.index());
        let mut targets = self.sequence_reference_targets(value);
        if targets.iter().next().is_none()
            && let Some(group) = self
                .typed
                .exprs
                .target_group_of(expression)
                .cloned()
                .flatten()
        {
            targets.extend(&group);
        }
        (targets, value.loans.clone())
    }

    pub(super) fn validate_sequence_borrow(&mut self, iterable: ExprId, statements: &[ExprId]) {
        let ty = &self.typed.exprs.ty[iterable.index()];
        let permission =
            crate::ty::reference_semantics_for_type(ty, Some(&self.typed.type_registry))
                .filter(|reference| matches!(reference.pointee, Ty::Array { .. }))
                .map(|reference| reference.permission);
        if permission != Some(crate::type_registry::ReferencePermission::Mutate) {
            return;
        }
        let (targets, _) = self.sequence_access(iterable);
        let Some((item, body)) = statements.split_first() else {
            return;
        };
        let TypedExprKind::Bind { body: element, .. } = self.typed.exprs.kind[item.index()] else {
            return;
        };
        let loan = ReferenceLoan::new(element);
        let mut projection_bases = HashSet::new();
        for statement in body {
            let _ = crate::typed::walk_expr_preorder(self.typed, *statement, &mut |expression| {
                match self.typed.exprs.kind_of(expression) {
                    Some(TypedExprKind::BufGet { buf, .. } | TypedExprKind::BufSet { buf, .. }) => {
                        projection_bases.insert(*buf);
                    }
                    Some(TypedExprKind::AggregateGet { base, .. }) => {
                        projection_bases.insert(*base);
                    }
                    _ => {}
                }
                std::ops::ControlFlow::Continue(())
            });
        }
        let mut conflicts = HashSet::new();
        for statement in body {
            let _ = crate::typed::walk_expr_preorder(self.typed, *statement, &mut |expression| {
                if projection_bases.contains(&expression) {
                    return std::ops::ControlFlow::Continue(());
                }
                let access = match self.typed.exprs.kind_of(expression) {
                    Some(
                        TypedExprKind::FnCall { args: None, .. } | TypedExprKind::SelfRef { .. },
                    ) => expression,
                    Some(TypedExprKind::BufGet { buf, .. } | TypedExprKind::BufSet { buf, .. }) => {
                        *buf
                    }
                    Some(
                        TypedExprKind::AggregateGet { .. }
                        | TypedExprKind::AggregateSet { .. }
                        | TypedExprKind::RecordSet { .. },
                    ) => expression,
                    Some(TypedExprKind::Loop(crate::typed::TypedLoop {
                        kind:
                            TypedLoopKind::ForIn {
                                iterable,
                                traversal: crate::typed::SequenceTraversal::RuntimeView { .. },
                            },
                        ..
                    })) => *iterable,
                    _ => return std::ops::ControlFlow::Continue(()),
                };
                let (access_targets, loans) = self.sequence_access(access);
                if !loans.contains(&loan)
                    && !targets.is_disjoint(&access_targets, &self.flow_ctx.constraint_env)
                {
                    conflicts.insert(expression);
                }
                std::ops::ControlFlow::Continue(())
            });
        }
        for expression in conflicts {
            // type-sequence-borrow-conflict
            let diagnostic = Diagnostic::new(
                SequenceCode::BorrowConflict,
                "mutable sequence traversal requires access through its current element",
            )
            .at_span_id(
                self.typed.exprs.span[expression.index()],
                &self.typed.span_table,
            );
            self.typed.exprs.facts.semantic_facts.flaws[expression.index()].push(diagnostic);
        }
    }
}
