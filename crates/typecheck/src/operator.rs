use ast::prelude::OperatorRole;

use crate::ty::Ty;
use crate::typed::{DefId, TypedCallableSignature};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorResolutionError {
    NoVisibleContract,
    OperandDoesNotProvide,
    Ambiguous,
}

pub struct OperatorRegistry {
    entries: Vec<(DefId, TypedCallableSignature)>,
}

impl OperatorRegistry {
    pub fn new(
        local: impl Iterator<Item = (DefId, TypedCallableSignature)>,
        visible: impl Iterator<Item = (DefId, TypedCallableSignature)>,
    ) -> Self {
        let mut entries: Vec<_> = local.chain(visible).collect();
        entries.sort_by(|(left, _), (right, _)| left.0.as_str().cmp(right.0.as_str()));
        entries.dedup_by_key(|(id, _)| *id);
        Self { entries }
    }

    pub fn resolve(
        &self,
        role: OperatorRole,
        lhs: &Ty,
        rhs: &Ty,
    ) -> Result<(DefId, TypedCallableSignature), OperatorResolutionError> {
        let registered: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, signature)| signature.operator_role == Some(role))
            .collect();
        if registered.is_empty() {
            return Err(OperatorResolutionError::NoVisibleContract);
        }
        if lhs.is_int() && rhs.is_int() {
            let applicable: Vec<_> = registered
                .iter()
                .filter(|(_, signature)| {
                    matches!(
                        signature.params.as_slice(),
                        [(_, left), (_, right)] if left == lhs && right == rhs
                    )
                })
                .collect();
            return match applicable.as_slice() {
                [(id, signature)] => Ok((*id, (*signature).clone())),
                [] => Err(OperatorResolutionError::OperandDoesNotProvide),
                _ => Err(OperatorResolutionError::Ambiguous),
            };
        }
        let left_matches: Vec<_> = registered
            .into_iter()
            .filter(|(_, signature)| {
                matches!(signature.params.as_slice(), [(_, left), (_, _)] if left == lhs)
            })
            .collect();
        if left_matches.is_empty() {
            return Err(OperatorResolutionError::NoVisibleContract);
        }
        let applicable: Vec<_> = left_matches
            .into_iter()
            .filter(|(_, signature)| {
                matches!(
                    signature.params.as_slice(),
                    [(_, left), (_, right)] if left == lhs && right == rhs
                )
            })
            .collect();
        match applicable.as_slice() {
            [(id, signature)] => Ok((*id, (*signature).clone())),
            [] => Err(OperatorResolutionError::OperandDoesNotProvide),
            _ => Err(OperatorResolutionError::Ambiguous),
        }
    }
}

#[cfg(test)]
#[path = "../tests/operator_tests.rs"]
mod tests;
