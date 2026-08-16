use std::collections::HashMap;

use ast::{BinderId, DependentArgId, NormalExpr, Ty, TyArg, UnionVariant};
use internment::Intern;

pub struct DependentInstantiation {
    source: BinderId,
    target: BinderId,
    renamed: HashMap<DependentArgId, DependentArgId>,
}

impl DependentInstantiation {
    pub fn new(source: BinderId, target: BinderId) -> Self {
        Self {
            source,
            target,
            renamed: HashMap::new(),
        }
    }

    pub fn apply_to_ty(&mut self, ty: &Ty) -> Ty {
        match ty {
            Ty::Named { instance, name } => Ty::Named {
                instance: instance.clone().with_arguments(
                    instance
                        .arguments
                        .iter()
                        .map(|(name, arg)| (*name, self.apply_to_arg(arg)))
                        .collect(),
                ),
                name: *name,
            },
            Ty::Record {
                name,
                fields,
                resolved_params,
            } => Ty::Record {
                name: *name,
                fields: fields
                    .iter()
                    .map(|(name, ty)| (*name, Box::new(self.apply_to_ty(ty))))
                    .collect(),
                resolved_params: resolved_params.as_ref().map(|params| {
                    params
                        .iter()
                        .map(|(name, arg)| (*name, self.apply_to_arg(arg)))
                        .collect()
                }),
            },
            Ty::Union {
                name,
                variants,
                literal_values,
                resolved_params,
            } => Ty::Union {
                name: *name,
                variants: variants
                    .iter()
                    .map(|variant| UnionVariant {
                        name: variant.name,
                        fields: variant
                            .fields
                            .iter()
                            .map(|(name, ty)| (*name, Box::new(self.apply_to_ty(ty))))
                            .collect(),
                        result_ty: variant.result_ty.as_ref().map(|ty| self.apply_to_ty(ty)),
                    })
                    .collect(),
                literal_values: literal_values.clone(),
                resolved_params: resolved_params.as_ref().map(|params| {
                    params
                        .iter()
                        .map(|(name, arg)| (*name, self.apply_to_arg(arg)))
                        .collect()
                }),
            },
            Ty::Tuple(types) => Ty::Tuple(types.iter().map(|ty| self.apply_to_ty(ty)).collect()),
            Ty::Array { elem, size } => Ty::Array {
                elem: Box::new(self.apply_to_ty(elem)),
                size: self.apply_to_normal(size),
            },
            Ty::Ptr { inner } => Ty::Ptr {
                inner: Box::new(self.apply_to_ty(inner)),
            },
            Ty::Address {
                pointee,
                address_space,
            } => Ty::Address {
                pointee: Box::new(self.apply_to_ty(pointee)),
                address_space: *address_space,
            },
            Ty::Ref { inner, mutable } => Ty::Ref {
                inner: Box::new(self.apply_to_ty(inner)),
                mutable: *mutable,
            },
            Ty::AnonymousInteger { .. }
            | Ty::ResultFamily { .. }
            | Ty::Float { .. }
            | Ty::Unit
            | Ty::Opaque(_)
            | Ty::Literal(_)
            | Ty::UnresolvedLiteral(_) => ty.clone(),
        }
    }

    pub fn apply_to_arg(&mut self, arg: &TyArg) -> TyArg {
        match arg {
            TyArg::Type(ty) => TyArg::Type(Box::new(self.apply_to_ty(ty))),
            TyArg::Const(expr) => TyArg::Const(self.apply_to_normal(expr)),
        }
    }

    pub fn apply_to_normal(&mut self, expr: &NormalExpr) -> NormalExpr {
        match expr {
            NormalExpr::Inferred(id) if id.binder == self.source => {
                let renamed = *self
                    .renamed
                    .entry(*id)
                    .or_insert_with(|| DependentArgId::new(self.target, id.slot));
                NormalExpr::Inferred(renamed)
            }
            NormalExpr::Add(left, right) => NormalExpr::Add(
                Box::new(self.apply_to_normal(left)),
                Box::new(self.apply_to_normal(right)),
            ),
            NormalExpr::Sub(left, right) => NormalExpr::Sub(
                Box::new(self.apply_to_normal(left)),
                Box::new(self.apply_to_normal(right)),
            ),
            NormalExpr::Mul(left, right) => NormalExpr::Mul(
                Box::new(self.apply_to_normal(left)),
                Box::new(self.apply_to_normal(right)),
            ),
            NormalExpr::Value(_)
            | NormalExpr::Var(_)
            | NormalExpr::Inferred(_)
            | NormalExpr::TargetQuery { .. } => expr.clone(),
        }
    }
}

/// A substitution that tracks mappings for both type variables and const variables.
#[derive(Default)]
pub struct DepSubst {
    pub types: HashMap<Intern<String>, Ty>,
    pub consts: HashMap<Intern<String>, NormalExpr>,
}

impl DepSubst {
    pub fn new() -> Self {
        DepSubst {
            types: HashMap::new(),
            consts: HashMap::new(),
        }
    }

    /// Build a substitution from a slice of (name, TyArg) pairs.
    pub fn from_ty_args(args: &[(Intern<String>, TyArg)]) -> Self {
        let mut s = Self::new();
        for (name, arg) in args {
            match arg {
                TyArg::Type(ty) => {
                    s.types.insert(*name, *ty.clone());
                }
                TyArg::Const(cv) => {
                    s.consts.insert(*name, cv.clone());
                }
            }
        }
        s
    }

    pub fn from_maps(
        types: HashMap<Intern<String>, Ty>,
        consts: HashMap<Intern<String>, NormalExpr>,
    ) -> Self {
        DepSubst { types, consts }
    }

    /// Merge another substitution into this one.
    pub fn extend(&mut self, other: DepSubst) {
        self.types.extend(other.types);
        self.consts.extend(other.consts);
    }

    /// Apply this substitution to a [`Ty`], replacing type variables and
    /// substituting const variables in [`NormalExpr`] positions.
    pub fn apply_to_ty(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Named { instance, name } => Ty::Named {
                instance: instance.clone().with_arguments(
                    instance
                        .arguments
                        .iter()
                        .map(|(name, arg)| {
                            let arg = match arg {
                                TyArg::Type(ty) => TyArg::Type(Box::new(self.apply_to_ty(ty))),
                                TyArg::Const(expr) => TyArg::Const(self.apply_to_normal(expr)),
                            };
                            (*name, arg)
                        })
                        .collect(),
                ),
                name: *name,
            },
            Ty::Opaque(name) => self.types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
            Ty::Record {
                name,
                fields,
                resolved_params,
            } => Ty::Record {
                name: *name,
                fields: fields
                    .iter()
                    .map(|(n, t)| (*n, Box::new(self.apply_to_ty(t))))
                    .collect(),
                resolved_params: resolved_params.as_ref().map(|params| {
                    params
                        .iter()
                        .map(|(name, arg)| {
                            let arg = match arg {
                                TyArg::Type(ty) => TyArg::Type(Box::new(self.apply_to_ty(ty))),
                                TyArg::Const(expr) => TyArg::Const(self.apply_to_normal(expr)),
                            };
                            (*name, arg)
                        })
                        .collect()
                }),
            },
            Ty::Union {
                name,
                variants,
                literal_values,
                resolved_params,
            } => Ty::Union {
                name: *name,
                variants: variants
                    .iter()
                    .map(|v| {
                        let new_fields = v
                            .fields
                            .iter()
                            .map(|(n, t)| (*n, Box::new(self.apply_to_ty(t))))
                            .collect();
                        UnionVariant::new(v.name, new_fields)
                    })
                    .collect(),
                literal_values: literal_values.clone(),
                resolved_params: resolved_params.as_ref().map(|params| {
                    params
                        .iter()
                        .map(|(name, arg)| {
                            let arg = match arg {
                                TyArg::Type(ty) => TyArg::Type(Box::new(self.apply_to_ty(ty))),
                                TyArg::Const(expr) => TyArg::Const(self.apply_to_normal(expr)),
                            };
                            (*name, arg)
                        })
                        .collect()
                }),
            },
            Ty::Literal(v) => Ty::Literal(v.clone()),
            Ty::ResultFamily { .. } => ty.clone(),
            Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|t| self.apply_to_ty(t)).collect()),
            Ty::Array { elem, size } => Ty::Array {
                elem: Box::new(self.apply_to_ty(elem)),
                size: self.apply_to_normal(size),
            },
            Ty::Ptr { inner } => Ty::Ptr {
                inner: Box::new(self.apply_to_ty(inner)),
            },
            Ty::Address {
                pointee,
                address_space,
            } => Ty::Address {
                pointee: Box::new(self.apply_to_ty(pointee)),
                address_space: *address_space,
            },
            Ty::Ref { inner, mutable } => Ty::Ref {
                inner: Box::new(self.apply_to_ty(inner)),
                mutable: *mutable,
            },
            Ty::AnonymousInteger { .. }
            | Ty::Float { .. }
            | Ty::Unit
            | Ty::UnresolvedLiteral(_) => ty.clone(),
        }
    }

    /// Apply this substitution to a [`NormalExpr`], replacing variable references.
    pub fn apply_to_normal(&self, expr: &NormalExpr) -> NormalExpr {
        match expr {
            NormalExpr::Value(_) | NormalExpr::Inferred(_) => expr.clone(),
            NormalExpr::Var(name) => self
                .consts
                .get(name)
                .cloned()
                .unwrap_or(NormalExpr::Var(*name)),
            NormalExpr::Add(l, r) => NormalExpr::Add(
                Box::new(self.apply_to_normal(l)),
                Box::new(self.apply_to_normal(r)),
            ),
            NormalExpr::Sub(l, r) => NormalExpr::Sub(
                Box::new(self.apply_to_normal(l)),
                Box::new(self.apply_to_normal(r)),
            ),
            NormalExpr::Mul(l, r) => NormalExpr::Mul(
                Box::new(self.apply_to_normal(l)),
                Box::new(self.apply_to_normal(r)),
            ),
            NormalExpr::TargetQuery { kind, operand } => NormalExpr::TargetQuery {
                kind: *kind,
                operand: self.apply_to_type_reference(operand),
            },
        }
    }

    fn apply_to_type_reference(&self, reference: &ast::TypeReference) -> ast::TypeReference {
        match reference {
            ast::TypeReference::Nominal(name) => self
                .types
                .get(name)
                .and_then(type_reference_for_ty)
                .unwrap_or_else(|| reference.clone()),
            ast::TypeReference::Application { name, arguments } => {
                ast::TypeReference::Application {
                    name: *name,
                    arguments: arguments
                        .iter()
                        .map(|argument| self.apply_to_type_reference(argument))
                        .collect(),
                }
            }
            ast::TypeReference::Pointer(inner) => {
                ast::TypeReference::Pointer(Box::new(self.apply_to_type_reference(inner)))
            }
            ast::TypeReference::Unit => ast::TypeReference::Unit,
        }
    }
}

fn type_reference_for_ty(ty: &Ty) -> Option<ast::TypeReference> {
    match ty {
        Ty::Unit => Some(ast::TypeReference::Unit),
        Ty::Opaque(name) => Some(ast::TypeReference::Nominal(*name)),
        Ty::Ptr { inner } => Some(ast::TypeReference::Pointer(Box::new(
            type_reference_for_ty(inner)?,
        ))),
        Ty::Named { name, instance, .. } => Some(ast::TypeReference::Application {
            name: *name,
            arguments: instance
                .arguments
                .iter()
                .filter_map(|(_, argument)| match argument {
                    TyArg::Type(ty) => type_reference_for_ty(ty),
                    TyArg::Const(_) => None,
                })
                .collect(),
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../tests/subst_tests.rs"]
mod tests;
