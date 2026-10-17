use std::collections::HashMap;
use std::sync::OnceLock;

use crate::ty::Ty;
use crate::typed::{DefId, TypedBind, TypedCallableSignature};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorResolutionError {
    NoVisibleContract,
    OperandDoesNotProvide,
    Ambiguous,
}

pub struct OperatorRegistry<'a> {
    local: Vec<(DefId, Option<Box<TypedCallableSignature>>)>,
    shared: Option<&'a HashMap<DefId, TypedCallableSignature>>,
    entries: OnceLock<Vec<(DefId, SignatureSource)>>,
}

enum SignatureSource {
    Local(usize),
    Shared(DefId),
}

impl OperatorRegistry<'static> {
    pub fn new(
        local: impl Iterator<Item = (DefId, TypedCallableSignature)>,
        visible: impl Iterator<Item = (DefId, TypedCallableSignature)>,
    ) -> Self {
        Self {
            local: local
                .chain(visible)
                .map(|(id, signature)| {
                    let signature = signature.infix_symbol.map(|_| Box::new(signature));
                    (id, signature)
                })
                .collect(),
            shared: None,
            entries: OnceLock::new(),
        }
    }
}

impl<'a> OperatorRegistry<'a> {
    pub(crate) fn with_shared_signatures(
        local: &HashMap<DefId, TypedBind>,
        shared: &'a HashMap<DefId, TypedCallableSignature>,
    ) -> Self {
        Self {
            local: local
                .iter()
                .map(|(id, bind)| {
                    let signature = bind
                        .infix_symbol
                        .map(|_| Box::new(TypedCallableSignature::from(bind)));
                    (*id, signature)
                })
                .collect(),
            shared: Some(shared),
            entries: OnceLock::new(),
        }
    }

    fn entries(&self) -> &[(DefId, SignatureSource)] {
        self.entries.get_or_init(|| {
            let mut entries: Vec<_> =
                self.local
                    .iter()
                    .enumerate()
                    .map(|(index, (id, _))| (*id, SignatureSource::Local(index)))
                    .chain(self.shared.into_iter().flat_map(|shared| {
                        shared.keys().map(|id| (*id, SignatureSource::Shared(*id)))
                    }))
                    .collect();
            entries.sort_by(|(left, _), (right, _)| left.0.as_str().cmp(right.0.as_str()));
            entries.dedup_by_key(|(id, _)| *id);
            entries.retain(|(_, source)| {
                self.signature(source)
                    .is_some_and(|signature| signature.infix_symbol.is_some())
            });
            entries
        })
    }

    fn signature(&self, source: &SignatureSource) -> Option<&TypedCallableSignature> {
        match source {
            SignatureSource::Local(index) => self.local[*index].1.as_deref(),
            SignatureSource::Shared(id) => self.shared?.get(id),
        }
    }

    pub fn resolve(
        &self,
        symbol: &str,
        lhs: &Ty,
        rhs: &Ty,
    ) -> Result<(DefId, TypedCallableSignature), OperatorResolutionError> {
        let registered: Vec<_> = self
            .entries()
            .iter()
            .filter_map(|(id, source)| {
                let signature = self.signature(source)?;
                signature
                    .infix_symbol
                    .as_ref()
                    .is_some_and(|registered| registered.as_str() == symbol)
                    .then_some((*id, signature))
            })
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
                .copied()
                .collect();
            return match applicable.as_slice() {
                [(id, signature)] => Ok((*id, (**signature).clone())),
                [] => Err(OperatorResolutionError::OperandDoesNotProvide),
                _ => Err(OperatorResolutionError::Ambiguous),
            };
        }
        let left_matches: Vec<_> = registered
            .into_iter()
            .filter(|(_, signature)| {
                matches!(signature.params.as_slice(), [(_, left), (_, _)] if matches_operands(&[left], &[lhs]))
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
                    [(_, left), (_, right)] if matches_operands(&[left, right], &[lhs, rhs])
                )
            })
            .collect();
        let exact: Vec<_> = applicable
            .iter()
            .copied()
            .filter(|(_, signature)| {
                matches!(signature.params.as_slice(), [(_, left), (_, right)] if left == lhs && right == rhs)
            })
            .collect();
        let applicable = if exact.is_empty() { applicable } else { exact };
        match applicable.as_slice() {
            [(id, signature)] => {
                let mut signature = (**signature).clone();
                if let [(_, left), (_, right)] = signature.params.as_slice()
                    && let Ok(subst) = crate::analysis::unify_type_args_with_registry(
                        &[
                            ast::TyArg::Type(Box::new(left.clone())),
                            ast::TyArg::Type(Box::new(right.clone())),
                        ],
                        &[
                            ast::TyArg::Type(Box::new(lhs.clone())),
                            ast::TyArg::Type(Box::new(rhs.clone())),
                        ],
                        None,
                    )
                {
                    signature.return_type = subst.apply_to_ty(&signature.return_type);
                    for (_, ty) in &mut signature.params {
                        *ty = subst.apply_to_ty(ty);
                    }
                    for result in &mut signature.results {
                        result.ty = subst.apply_to_ty(&result.ty);
                    }
                }
                Ok((*id, signature))
            }
            [] => Err(OperatorResolutionError::OperandDoesNotProvide),
            _ => Err(OperatorResolutionError::Ambiguous),
        }
    }
}

fn matches_operands(expected: &[&Ty], actual: &[&Ty]) -> bool {
    if expected == actual {
        return true;
    }
    let arguments = |types: &[&Ty]| {
        types
            .iter()
            .map(|ty| ast::TyArg::Type(Box::new((*ty).clone())))
            .collect::<Vec<_>>()
    };
    crate::analysis::unify_type_args_with_registry(&arguments(expected), &arguments(actual), None)
        .is_ok_and(|subst| !subst.types.is_empty() || !subst.consts.is_empty())
}

#[cfg(test)]
#[path = "operator/shared_tests.rs"]
mod shared_tests;
