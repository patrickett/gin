use std::collections::HashMap;

use ast::{BinderId, ConstExpr, DependentArgId, Ty, TyArg, UnionVariant};
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
                size: self.apply_to_const(size),
            },
            Ty::Ptr { inner } => Ty::Ptr {
                inner: Box::new(self.apply_to_ty(inner)),
            },
            Ty::Ref { inner, mutable } => Ty::Ref {
                inner: Box::new(self.apply_to_ty(inner)),
                mutable: *mutable,
            },
            Ty::Int { .. } | Ty::Float { .. } | Ty::Unit | Ty::Opaque(_) | Ty::Literal(_) => {
                ty.clone()
            }
        }
    }

    pub fn apply_to_arg(&mut self, arg: &TyArg) -> TyArg {
        match arg {
            TyArg::Type(ty) => TyArg::Type(Box::new(self.apply_to_ty(ty))),
            TyArg::Const(expr) => TyArg::Const(self.apply_to_const(expr)),
        }
    }

    pub fn apply_to_const(&mut self, expr: &ConstExpr) -> ConstExpr {
        match expr {
            ConstExpr::Inferred(id) if id.binder == self.source => {
                let renamed = *self
                    .renamed
                    .entry(*id)
                    .or_insert_with(|| DependentArgId::new(self.target, id.slot));
                ConstExpr::Inferred(renamed)
            }
            ConstExpr::Add(left, right) => ConstExpr::Add(
                Box::new(self.apply_to_const(left)),
                Box::new(self.apply_to_const(right)),
            ),
            ConstExpr::Sub(left, right) => ConstExpr::Sub(
                Box::new(self.apply_to_const(left)),
                Box::new(self.apply_to_const(right)),
            ),
            ConstExpr::Mul(left, right) => ConstExpr::Mul(
                Box::new(self.apply_to_const(left)),
                Box::new(self.apply_to_const(right)),
            ),
            ConstExpr::Value(_) | ConstExpr::Var(_) | ConstExpr::Inferred(_) => expr.clone(),
        }
    }
}

/// A substitution that tracks mappings for both type variables and const variables.
#[derive(Default)]
pub struct DepSubst {
    pub types: HashMap<Intern<String>, Ty>,
    pub consts: HashMap<Intern<String>, ConstExpr>,
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
        consts: HashMap<Intern<String>, ConstExpr>,
    ) -> Self {
        DepSubst { types, consts }
    }

    /// Merge another substitution into this one.
    pub fn extend(&mut self, other: DepSubst) {
        self.types.extend(other.types);
        self.consts.extend(other.consts);
    }

    /// Apply this substitution to a [`Ty`], replacing type variables and
    /// substituting const variables in [`ConstExpr`] positions.
    pub fn apply_to_ty(&self, ty: &Ty) -> Ty {
        match ty {
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
                                TyArg::Const(expr) => TyArg::Const(self.apply_to_const(expr)),
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
                                TyArg::Const(expr) => TyArg::Const(self.apply_to_const(expr)),
                            };
                            (*name, arg)
                        })
                        .collect()
                }),
            },
            Ty::Literal(v) => Ty::Literal(v.clone()),
            Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|t| self.apply_to_ty(t)).collect()),
            Ty::Array { elem, size } => Ty::Array {
                elem: Box::new(self.apply_to_ty(elem)),
                size: self.apply_to_const(size),
            },
            Ty::Ptr { inner } => Ty::Ptr {
                inner: Box::new(self.apply_to_ty(inner)),
            },
            Ty::Ref { inner, mutable } => Ty::Ref {
                inner: Box::new(self.apply_to_ty(inner)),
                mutable: *mutable,
            },
            Ty::Int { .. } | Ty::Float { .. } | Ty::Unit => ty.clone(),
        }
    }

    /// Apply this substitution to a [`ConstExpr`], replacing variable references.
    pub fn apply_to_const(&self, expr: &ConstExpr) -> ConstExpr {
        match expr {
            ConstExpr::Value(_) | ConstExpr::Inferred(_) => expr.clone(),
            ConstExpr::Var(name) => self
                .consts
                .get(name)
                .cloned()
                .unwrap_or(ConstExpr::Var(*name)),
            ConstExpr::Add(l, r) => ConstExpr::Add(
                Box::new(self.apply_to_const(l)),
                Box::new(self.apply_to_const(r)),
            ),
            ConstExpr::Sub(l, r) => ConstExpr::Sub(
                Box::new(self.apply_to_const(l)),
                Box::new(self.apply_to_const(r)),
            ),
            ConstExpr::Mul(l, r) => ConstExpr::Mul(
                Box::new(self.apply_to_const(l)),
                Box::new(self.apply_to_const(r)),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::{BinderId, BinderOwner, ConstExpr, ConstValue, DependentArgId, Ty};
    use internment::Intern;

    #[test]
    fn new_creates_empty_subst() {
        let subst = DepSubst::new();
        assert!(subst.types.is_empty());
        assert!(subst.consts.is_empty());
    }

    #[test]
    fn apply_to_ty_replaces_opaque() {
        let mut subst = DepSubst::new();
        subst.types.insert(
            Intern::from_ref("T"),
            Ty::Int {
                width: 64,
                signed: true,
                value: None,
                min: None,
                max: None,
            },
        );
        let ty = Ty::Opaque(Intern::from_ref("T"));
        let result = subst.apply_to_ty(&ty);
        assert!(result.is_signed_int());
    }

    #[test]
    fn apply_to_ty_leaves_unmatched_opaque() {
        let subst = DepSubst::new();
        let ty = Ty::Opaque(Intern::from_ref("T"));
        assert_eq!(subst.apply_to_ty(&ty), ty);
    }

    #[test]
    fn apply_to_ty_traverses_record_fields() {
        let mut subst = DepSubst::new();
        subst.types.insert(
            Intern::from_ref("T"),
            Ty::Int {
                width: 32,
                signed: false,
                value: None,
                min: None,
                max: None,
            },
        );
        let ty = Ty::Record {
            name: Intern::from_ref("Pair"),
            resolved_params: Some(vec![(
                Intern::from_ref("T"),
                TyArg::Type(Box::new(Ty::Opaque(Intern::from_ref("T")))),
            )]),
            fields: vec![(
                Intern::from_ref("first"),
                Box::new(Ty::Opaque(Intern::from_ref("T"))),
            )],
        };
        let result = subst.apply_to_ty(&ty);
        if let Ty::Record {
            fields,
            resolved_params: Some(params),
            ..
        } = &result
        {
            assert!(fields[0].1.is_unsigned_int());
            assert!(matches!(&params[0].1, TyArg::Type(ty) if ty.is_unsigned_int()));
        } else {
            panic!("expected Record");
        }
    }

    #[test]
    fn apply_to_ty_substitutes_array_size() {
        let mut subst = DepSubst::new();
        subst
            .consts
            .insert(Intern::from_ref("n"), ConstExpr::Value(ConstValue::Int(8)));
        let ty = Ty::Array {
            elem: Box::new(Ty::u8()),
            size: ConstExpr::Var(Intern::from_ref("n")),
        };
        let result = subst.apply_to_ty(&ty);
        if let Ty::Array { size, .. } = &result {
            assert_eq!(*size, ConstExpr::Value(ConstValue::Int(8)));
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn apply_to_ty_traverses_union_variants() {
        let mut subst = DepSubst::new();
        subst.types.insert(
            Intern::from_ref("T"),
            Ty::Int {
                width: 16,
                signed: true,
                value: None,
                min: None,
                max: None,
            },
        );
        let ty = Ty::Union {
            name: Intern::from_ref("Maybe"),
            variants: vec![UnionVariant::new(
                Intern::from_ref("Some"),
                vec![(
                    Intern::from_ref("value"),
                    Box::new(Ty::Opaque(Intern::from_ref("T"))),
                )],
            )],
            literal_values: None,
            resolved_params: None,
        };
        let result = subst.apply_to_ty(&ty);
        if let Ty::Union { variants, .. } = &result {
            assert!(variants[0].fields[0].1.is_signed_int());
        } else {
            panic!("expected Union");
        }
    }

    #[test]
    fn apply_to_ty_traverses_tuple() {
        let mut subst = DepSubst::new();
        subst.types.insert(
            Intern::from_ref("T"),
            Ty::Int {
                width: 8,
                signed: false,
                value: None,
                min: None,
                max: None,
            },
        );
        let ty = Ty::Tuple(vec![Ty::Opaque(Intern::from_ref("T"))]);
        let result = subst.apply_to_ty(&ty);
        if let Ty::Tuple(elems) = &result {
            assert!(elems[0].is_unsigned_int());
        } else {
            panic!("expected Tuple");
        }
    }

    #[test]
    fn apply_to_const_replaces_var() {
        let mut subst = DepSubst::new();
        subst
            .consts
            .insert(Intern::from_ref("n"), ConstExpr::Value(ConstValue::Int(42)));
        let expr = ConstExpr::Var(Intern::from_ref("n"));
        assert_eq!(
            subst.apply_to_const(&expr),
            ConstExpr::Value(ConstValue::Int(42))
        );
    }

    #[test]
    fn apply_to_const_leaves_unmatched_var() {
        let subst = DepSubst::new();
        let expr = ConstExpr::Var(Intern::from_ref("n"));
        assert_eq!(subst.apply_to_const(&expr), expr);
    }

    #[test]
    fn apply_to_const_traverses_add() {
        let mut subst = DepSubst::new();
        subst
            .consts
            .insert(Intern::from_ref("n"), ConstExpr::Value(ConstValue::Int(10)));
        let expr = ConstExpr::Add(
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
            Box::new(ConstExpr::Value(ConstValue::Int(5))),
        );
        let result = subst.apply_to_const(&expr);
        assert_eq!(
            result,
            ConstExpr::Add(
                Box::new(ConstExpr::Value(ConstValue::Int(10))),
                Box::new(ConstExpr::Value(ConstValue::Int(5))),
            )
        );
    }

    #[test]
    fn extend_merges_both_maps() {
        let mut a = DepSubst::new();
        a.types.insert(
            Intern::from_ref("T"),
            Ty::Int {
                width: 64,
                signed: true,
                value: None,
                min: None,
                max: None,
            },
        );
        let mut b = DepSubst::new();
        b.consts
            .insert(Intern::from_ref("n"), ConstExpr::Value(ConstValue::Int(0)));
        a.extend(b);
        assert_eq!(a.types.len(), 1);
        assert_eq!(a.consts.len(), 1);
    }

    #[test]
    fn apply_to_ty_leaves_primitives_unchanged() {
        let subst = DepSubst::new();
        let int_ty = Ty::Int {
            width: 64,
            signed: true,
            value: None,
            min: None,
            max: None,
        };
        assert_eq!(subst.apply_to_ty(&int_ty), int_ty);
        assert_eq!(subst.apply_to_ty(&Ty::Unit), Ty::Unit);
    }

    #[test]
    fn apply_to_const_leaves_value_unchanged() {
        let subst = DepSubst::new();
        let expr = ConstExpr::Value(ConstValue::Int(42));
        assert_eq!(subst.apply_to_const(&expr), expr);
    }

    #[test]
    fn ordinary_substitution_preserves_inferred_identity() {
        let mut subst = DepSubst::new();
        subst
            .consts
            .insert(Intern::from_ref("n"), ConstExpr::Value(ConstValue::Int(42)));
        let expr = ConstExpr::Inferred(DependentArgId::new(
            BinderId::new(2, BinderOwner::Expression(8)),
            1,
        ));

        assert_eq!(subst.apply_to_const(&expr), expr);
    }

    #[test]
    fn dependent_instantiation_shares_a_witness_within_one_call() {
        let callee = BinderId::new(1, BinderOwner::Definition(Intern::from_ref("callee")));
        let caller = BinderId::new(7, BinderOwner::Expression(12));
        let witness = ConstExpr::Inferred(DependentArgId::new(callee, 0));
        let input = Ty::Array {
            elem: Box::new(Ty::Unit),
            size: witness.clone(),
        };
        let output = Ty::Record {
            name: Intern::from_ref("Box"),
            fields: vec![(Intern::from_ref("items"), Box::new(input.clone()))],
            resolved_params: Some(vec![(Intern::from_ref("n"), TyArg::Const(witness))]),
        };
        let mut instantiation = DependentInstantiation::new(callee, caller);

        let input = instantiation.apply_to_ty(&input);
        let output = instantiation.apply_to_ty(&output);
        let Ty::Array {
            size: input_size, ..
        } = input
        else {
            panic!("expected array");
        };
        let Ty::Record {
            resolved_params: Some(params),
            ..
        } = output
        else {
            panic!("expected resolved record");
        };

        assert_eq!(params[0].1, TyArg::Const(input_size));
    }

    #[test]
    fn dependent_instantiation_is_distinct_per_call_and_preserves_other_consts() {
        let callee = BinderId::new(1, BinderOwner::Definition(Intern::from_ref("callee")));
        let witness = ConstExpr::Inferred(DependentArgId::new(callee, 0));
        let explicit = ConstExpr::Value(ConstValue::Int(4));
        let symbolic = ConstExpr::Var(Intern::from_ref("n"));
        let mut first =
            DependentInstantiation::new(callee, BinderId::new(7, BinderOwner::Expression(12)));
        let mut second =
            DependentInstantiation::new(callee, BinderId::new(7, BinderOwner::Expression(19)));

        assert_ne!(
            first.apply_to_const(&witness),
            second.apply_to_const(&witness)
        );
        assert_eq!(first.apply_to_const(&explicit), explicit);
        assert_eq!(first.apply_to_const(&symbolic), symbolic);
    }

    #[test]
    fn type_substitution_preserves_inferred_resolved_param() {
        let expr = ConstExpr::Inferred(DependentArgId::new(
            BinderId::new(2, BinderOwner::Definition(Intern::from_ref("Buffer"))),
            0,
        ));
        let ty = Ty::Record {
            name: Intern::from_ref("Buffer"),
            fields: Vec::new(),
            resolved_params: Some(vec![(Intern::from_ref("n"), TyArg::Const(expr.clone()))]),
        };

        let result = DepSubst::new().apply_to_ty(&ty);
        assert!(matches!(
            result,
            Ty::Record { resolved_params: Some(params), .. }
                if params == vec![(Intern::from_ref("n"), TyArg::Const(expr))]
        ));
    }
}
