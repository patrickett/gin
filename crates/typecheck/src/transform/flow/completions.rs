use super::*;
use crate::completions::{CompletionCandidate, CompletionKind};
use diagnostic::DiagnosticCode;
use diagnostic::code::{ProofCode, TypeCode};

impl WalkExprCtx<'_> {
    pub(super) fn probe_member_completions(&mut self, receiver: ExprId) {
        let Some((start, end)) = self.typed.completion_probe else {
            return;
        };
        let span = self
            .typed
            .span_table
            .get(self.typed.exprs.span[receiver.index()]);
        if span.start() != start || span.end() != end {
            return;
        }
        let Some(ty) = self.typed.exprs.ty_of(receiver) else {
            return;
        };
        let Some(owner) = ty.named_instance_stripping_reference_wrappers() else {
            return;
        };
        if self
            .typed
            .exprs
            .place_of(receiver)
            .flatten()
            .is_some_and(|place| {
                ownership_state(self.typed, self.flow_ctx, place, true)
                    .is_some_and(|state| state != VarState::Alive)
            })
        {
            return;
        }
        let mut candidates = std::collections::BTreeMap::new();
        if let Ty::Record { fields, .. } = self
            .typed
            .type_registry
            .resolved_definition_for_type(ty.without_reference_wrappers())
        {
            for (name, field_ty) in fields {
                if self.typed.can_access_member(ty, name) {
                    candidates.insert(
                        name.as_str().to_owned(),
                        CompletionCandidate::new(name.as_str(), CompletionKind::Variable)
                            .with_detail(crate::typed::format_ty_for_hover(&field_ty)),
                    );
                }
            }
        }
        for (id, signature) in self.callable_signatures.iter() {
            let Some((name, receiver_ty)) = signature.params.first() else {
                continue;
            };
            if name.as_str() != "self"
                || id.0.contains("$specialized$")
                || !self.typed.can_access_method(*id)
                || receiver_ty
                    .named_instance_stripping_reference_wrappers()
                    .is_none_or(|actual| actual.declaration != owner.declaration)
            {
                continue;
            }
            let flaws = validate_call_refinements(
                self.typed,
                CallRefinementInputs {
                    args: &[receiver],
                    argument_values: &[None],
                    signature,
                    substituted_return: None,
                },
                &self.flow_ctx.constraint_env,
                self.callable_signatures,
                self.local_const_values,
            );
            if flaws.iter().any(|flaw| {
                flaw.code
                    == DiagnosticCode::Type(TypeCode::Proof(ProofCode::ParameterRefinementFailed))
            }) {
                continue;
            }
            let label = id.0.as_str().rsplit('.').next().unwrap_or(id.0.as_str());
            candidates.insert(
                label.to_owned(),
                CompletionCandidate::new(label, CompletionKind::Function)
                    .with_detail(crate::typed::format_ty_for_hover(&signature.return_type)),
            );
        }
        for candidate in candidates.into_values() {
            if !self
                .typed
                .member_completions
                .iter()
                .any(|old| old.label == candidate.label)
            {
                self.typed.member_completions.push(candidate);
            }
        }
    }
}
