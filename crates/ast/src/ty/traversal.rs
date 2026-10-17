use std::ops::ControlFlow;

use super::{SequenceTypeElement, Ty, TyArg, UnionVariant};
use crate::NormalExpr;

/// Provides the replacement for each direct child while rebuilding a [`Ty`].
///
/// Implementations decide leaf policy; the structural match lives once in
/// [`Ty::map_children`].
pub trait TyChildMapper {
    fn map_ty(&mut self, ty: &Ty) -> Ty;
    fn map_arg(&mut self, arg: &TyArg) -> TyArg;
    fn map_constant(&mut self, expr: &NormalExpr) -> NormalExpr;
}

#[derive(Clone, Copy)]
pub enum TyChild<'a> {
    Type(&'a Ty),
    Argument(&'a TyArg),
    Constant(&'a NormalExpr),
}

impl<'a> TyChild<'a> {
    pub fn ty(self) -> Option<&'a Ty> {
        match self {
            Self::Type(ty) => Some(ty),
            Self::Argument(TyArg::Type(ty)) => Some(ty),
            Self::Argument(TyArg::Const(_)) | Self::Constant(_) => None,
        }
    }
}

impl Ty {
    /// Rebuilds this type with every direct child passed through `mapper`.
    ///
    /// The match is exhaustive: adding a [`Ty`] variant fails compilation here,
    /// keeping all structural rewrites in one place.
    pub fn map_children<M: TyChildMapper + ?Sized>(&self, mapper: &mut M) -> Ty {
        match self {
            Self::Named { instance, name } => Ty::Named {
                instance: instance.clone().with_arguments(
                    instance
                        .arguments
                        .iter()
                        .map(|(name, arg)| (*name, mapper.map_arg(arg)))
                        .collect(),
                ),
                name: *name,
            },
            Self::Record {
                name,
                fields,
                resolved_params,
            } => Ty::Record {
                name: *name,
                fields: fields
                    .iter()
                    .map(|(n, t)| (*n, Box::new(mapper.map_ty(t))))
                    .collect(),
                resolved_params: resolved_params.as_ref().map(|params| {
                    params
                        .iter()
                        .map(|(n, arg)| (*n, mapper.map_arg(arg)))
                        .collect()
                }),
            },
            Self::Union {
                name,
                variants,
                literal_values,
                resolved_params,
            } => Ty::Union {
                name: *name,
                variants: variants
                    .iter()
                    .map(|variant| UnionVariant {
                        kind: variant.kind,
                        discriminant: variant.discriminant,
                        name: variant.name,
                        fields: variant
                            .fields
                            .iter()
                            .map(|(n, t)| (*n, Box::new(mapper.map_ty(t))))
                            .collect(),
                        result_ty: variant.result_ty.as_ref().map(|t| mapper.map_ty(t)),
                    })
                    .collect(),
                literal_values: literal_values.clone(),
                resolved_params: resolved_params.as_ref().map(|params| {
                    params
                        .iter()
                        .map(|(n, arg)| (*n, mapper.map_arg(arg)))
                        .collect()
                }),
            },
            Self::Sequence(elements) => Ty::Sequence(
                elements
                    .iter()
                    .map(|element| SequenceTypeElement {
                        name: element.name,
                        ty: Box::new(mapper.map_ty(&element.ty)),
                    })
                    .collect(),
            ),
            Self::Array { elem, size } => Ty::Array {
                elem: Box::new(mapper.map_ty(elem)),
                size: mapper.map_constant(size),
            },
            Self::UnresolvedSequence { element, length } => Ty::UnresolvedSequence {
                element: element
                    .as_deref()
                    .map(|element| Box::new(mapper.map_ty(element))),
                length: mapper.map_constant(length),
            },
            Self::Ref { inner, mutable } => Ty::Ref {
                inner: Box::new(mapper.map_ty(inner)),
                mutable: *mutable,
            },
            Self::Ptr { inner } => Ty::Ptr {
                inner: Box::new(mapper.map_ty(inner)),
            },
            Self::OwnedStorage { inner } => Ty::OwnedStorage {
                inner: Box::new(mapper.map_ty(inner)),
            },
            Self::Address {
                pointee,
                address_space,
            } => Ty::Address {
                pointee: Box::new(mapper.map_ty(pointee)),
                address_space: *address_space,
            },
            Self::Proposition(_)
            | Self::Opaque(_)
            | Self::AnonymousInteger { .. }
            | Self::ResultFamily { .. }
            | Self::Float { .. }
            | Self::Unit
            | Self::Literal(_)
            | Self::UnresolvedLiteral(_) => self.clone(),
        }
    }

    pub fn any_child(&self, mut predicate: impl FnMut(TyChild<'_>) -> bool) -> bool {
        self.walk_children(&mut |child| {
            if predicate(child) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .is_break()
    }

    pub fn walk_children<'a, B>(
        &'a self,
        visit: &mut impl FnMut(TyChild<'a>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        match self {
            Self::Named { instance, .. } => {
                for (_, argument) in &instance.arguments {
                    visit(TyChild::Argument(argument))?;
                }
            }
            Self::Record { fields, .. } => {
                for (_, ty) in fields {
                    visit(TyChild::Type(ty))?;
                }
            }
            Self::Union { variants, .. } => {
                for variant in variants {
                    for (_, ty) in &variant.fields {
                        visit(TyChild::Type(ty))?;
                    }
                    if let Some(ty) = &variant.result_ty {
                        visit(TyChild::Type(ty))?;
                    }
                }
            }
            Self::Array { elem, size } => {
                visit(TyChild::Type(elem))?;
                visit(TyChild::Constant(size))?;
            }
            Self::UnresolvedSequence { element, length } => {
                if let Some(ty) = element {
                    visit(TyChild::Type(ty))?;
                }
                visit(TyChild::Constant(length))?;
            }
            Self::Ref { inner, .. }
            | Self::Ptr { inner }
            | Self::OwnedStorage { inner }
            | Self::Address { pointee: inner, .. } => {
                visit(TyChild::Type(inner))?;
            }
            Self::Sequence(elements) => {
                for element in elements {
                    visit(TyChild::Type(&element.ty))?;
                }
            }
            Self::Proposition(_)
            | Self::Opaque(_)
            | Self::AnonymousInteger { .. }
            | Self::ResultFamily { .. }
            | Self::Float { .. }
            | Self::Unit
            | Self::Literal(_)
            | Self::UnresolvedLiteral(_) => {}
        }
        if let Self::Record {
            resolved_params, ..
        }
        | Self::Union {
            resolved_params, ..
        } = self
        {
            for (_, argument) in resolved_params.iter().flatten() {
                visit(TyChild::Argument(argument))?;
            }
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::UnionVariant;
    use internment::Intern;

    #[test]
    fn union_children_preserve_order_and_stop_on_break() {
        let name = Intern::from_ref("T");
        let mut variant = UnionVariant::new(name, vec![(name, Box::new(Ty::Unit))]);
        variant.result_ty = Some(Ty::Opaque(name));
        let ty = Ty::Union {
            name,
            variants: vec![variant],
            literal_values: None,
            resolved_params: Some(vec![(name, TyArg::Const(NormalExpr::from(3)))]),
        };
        let mut visited = Vec::new();
        let result = ty.walk_children(&mut |child| {
            visited.push(match child {
                TyChild::Type(Ty::Unit) => "field",
                TyChild::Type(Ty::Opaque(_)) => "result",
                TyChild::Argument(TyArg::Const(_)) => "argument",
                _ => panic!("unexpected child"),
            });
            ControlFlow::<()>::Continue(())
        });
        assert!(result.is_continue());
        assert_eq!(visited, ["field", "result", "argument"]);
        let mut count = 0;
        let found = ty.walk_children(&mut |child| {
            count += 1;
            match child {
                TyChild::Type(Ty::Opaque(name)) => ControlFlow::Break(*name),
                _ => ControlFlow::Continue(()),
            }
        });
        assert_eq!(found, ControlFlow::Break(name));
        assert_eq!(count, 2);
    }
}
