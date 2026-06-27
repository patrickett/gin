use std::collections::HashMap;

use ast::{ConstExpr, Ty};
use internment::Intern;

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
            Ty::Record { name, fields } => Ty::Record {
                name: *name,
                fields: fields
                    .iter()
                    .map(|(n, t)| (*n, Box::new(self.apply_to_ty(t))))
                    .collect(),
            },
            Ty::Union {
                name,
                variants,
                literal_values,
            } => Ty::Union {
                name: *name,
                variants: variants
                    .iter()
                    .map(|(vn, fields)| {
                        let new_fields = fields
                            .iter()
                            .map(|(n, t)| (*n, Box::new(self.apply_to_ty(t))))
                            .collect();
                        (*vn, new_fields)
                    })
                    .collect(),
                literal_values: literal_values.clone(),
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
            ConstExpr::Value(_) => expr.clone(),
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
    use ast::{ConstExpr, ConstValue, Ty};
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
            fields: vec![(
                Intern::from_ref("first"),
                Box::new(Ty::Opaque(Intern::from_ref("T"))),
            )],
        };
        let result = subst.apply_to_ty(&ty);
        if let Ty::Record { fields, .. } = &result {
            assert!(fields[0].1.is_unsigned_int());
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
            variants: vec![(
                Intern::from_ref("Some"),
                vec![(
                    Intern::from_ref("value"),
                    Box::new(Ty::Opaque(Intern::from_ref("T"))),
                )],
            )],
            literal_values: None,
        };
        let result = subst.apply_to_ty(&ty);
        if let Ty::Union { variants, .. } = &result {
            assert!(variants[0].1[0].1.is_signed_int());
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
}
