use std::ops::ControlFlow;

use super::{Ty, TyArg};
use crate::NormalExpr;

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
            | Self::Address { pointee: inner, .. } => {
                visit(TyChild::Type(inner))?;
            }
            Self::Sequence(elements) => {
                for element in elements {
                    visit(TyChild::Type(&element.ty))?;
                }
            }
            Self::Opaque(_)
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
