use std::collections::HashMap;

use ast::{BinderId, DependentArgId, NormalExpr, Ty, TyArg};
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
        ty.map_children(self)
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

impl ast::ty::TyChildMapper for DependentInstantiation {
    fn map_ty(&mut self, ty: &Ty) -> Ty {
        self.apply_to_ty(ty)
    }
    fn map_arg(&mut self, arg: &TyArg) -> TyArg {
        self.apply_to_arg(arg)
    }
    fn map_constant(&mut self, expr: &NormalExpr) -> NormalExpr {
        self.apply_to_normal(expr)
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

    /// Apply this substitution to a [`Ty`], replacing type variables and
    /// substituting const variables in [`NormalExpr`] positions.
    pub fn apply_to_ty(&self, ty: &Ty) -> Ty {
        if let Ty::Opaque(name) = ty {
            return self.types.get(name).cloned().unwrap_or(Ty::Opaque(*name));
        }
        if let Ty::Proposition(predicate) = ty {
            return Ty::Proposition(Box::new(crate::analysis::propositions::substitute_names(
                predicate,
                |name| self.apply_to_normal(&NormalExpr::Var(name)),
            )));
        }
        ty.map_children(&mut DepSubstMapper(self))
    }

    fn apply_to_arg(&self, arg: &TyArg) -> TyArg {
        match arg {
            TyArg::Type(ty) => TyArg::Type(Box::new(self.apply_to_ty(ty))),
            TyArg::Const(expr) => TyArg::Const(self.apply_to_normal(expr)),
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

struct DepSubstMapper<'a>(&'a DepSubst);

impl ast::ty::TyChildMapper for DepSubstMapper<'_> {
    fn map_ty(&mut self, ty: &Ty) -> Ty {
        self.0.apply_to_ty(ty)
    }
    fn map_arg(&mut self, arg: &TyArg) -> TyArg {
        self.0.apply_to_arg(arg)
    }
    fn map_constant(&mut self, expr: &NormalExpr) -> NormalExpr {
        self.0.apply_to_normal(expr)
    }
}

pub(crate) fn type_reference_for_ty(ty: &Ty) -> Option<ast::TypeReference> {
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
                .map(|(_, argument)| match argument {
                    TyArg::Type(ty) => type_reference_for_ty(ty),
                    TyArg::Const(_) => None,
                })
                .collect::<Option<Vec<_>>>()?,
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../tests/subst/conversion.rs"]
mod tests;
