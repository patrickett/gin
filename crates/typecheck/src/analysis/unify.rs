use ast::{ConstExpr, Ty, TyArg};
use std::fmt::Write;

use crate::subst::DepSubst;

/// Walk two `TyArg` lists pairwise, unifying each pair and building a `DepSubst`.
pub fn unify_type_args(expected: &[TyArg], actual: &[TyArg]) -> Result<DepSubst, String> {
    if expected.len() != actual.len() {
        return Err(format!(
            "expected {} type arguments, found {}",
            expected.len(),
            actual.len()
        ));
    }
    let mut subst = DepSubst::new();
    for (exp, act) in expected.iter().zip(actual.iter()) {
        unify_ty_arg(exp, act, &mut subst)?;
    }
    Ok(subst)
}

fn unify_ty_arg(expected: &TyArg, actual: &TyArg, subst: &mut DepSubst) -> Result<(), String> {
    match (expected, actual) {
        (TyArg::Type(exp), TyArg::Type(act)) => unify_ty(exp, act, subst),
        (TyArg::Const(exp), TyArg::Const(act)) => unify_const(exp, act, subst),
        (TyArg::Type(_), _) => Err("expected type argument, found const".to_string()),
        (TyArg::Const(_), _) => Err("expected const argument, found type".to_string()),
    }
}

fn unify_ty(expected: &Ty, actual: &Ty, subst: &mut DepSubst) -> Result<(), String> {
    match (expected, actual) {
        (Ty::Opaque(name), _) => {
            if let Some(existing) = subst.types.get(name) {
                if existing != actual {
                    return Err(format!(
                        "type parameter `{}` inferred as both {} and {}",
                        name.as_str(),
                        existing.format_for_hover(),
                        actual.format_for_hover()
                    ));
                }
            } else {
                subst.types.insert(*name, actual.clone());
            }
            Ok(())
        }
        (Ty::Int { .. }, Ty::Int { .. })
        | (Ty::Float { .. }, Ty::Float { .. })
        | (Ty::Unit, Ty::Unit) => Ok(()),
        (Ty::Record { name: n1, .. }, Ty::Record { name: n2, .. }) if n1 == n2 => {
            if let (Ty::Record { fields: ef, .. }, Ty::Record { fields: af, .. }) =
                (expected, actual)
                && ef.len() == af.len()
            {
                for ((_, et), (_, at)) in ef.iter().zip(af.iter()) {
                    unify_ty(et, at, subst)?;
                }
            }
            Ok(())
        }
        (Ty::Union { name: n1, .. }, Ty::Union { name: n2, .. }) if n1 == n2 => Ok(()),
        (Ty::Tuple(e1), Ty::Tuple(e2)) if e1.len() == e2.len() => {
            for (et, at) in e1.iter().zip(e2.iter()) {
                unify_ty(et, at, subst)?;
            }
            Ok(())
        }
        (Ty::Array { elem: e1, size: s1 }, Ty::Array { elem: e2, size: s2 }) => {
            unify_ty(e1, e2, subst)?;
            unify_const(s1, s2, subst)
        }
        (Ty::Ptr { inner: i1 }, Ty::Ptr { inner: i2 })
        | (Ty::Ref { inner: i1, .. }, Ty::Ref { inner: i2, .. }) => unify_ty(i1, i2, subst),
        _ => {
            let mut msg = String::new();
            let _ = write!(
                msg,
                "type mismatch: expected {}, found {}",
                expected.format_for_hover(),
                actual.format_for_hover()
            );
            Err(msg)
        }
    }
}

fn unify_const(
    expected: &ConstExpr,
    actual: &ConstExpr,
    subst: &mut DepSubst,
) -> Result<(), String> {
    match (expected, actual) {
        (ConstExpr::Var(name), _) => {
            if let Some(existing) = subst.consts.get(name) {
                if existing != actual {
                    return Err(format!(
                        "const parameter `{}` inferred as both {} and {}",
                        name.as_str(),
                        existing,
                        actual
                    ));
                }
            } else {
                subst.consts.insert(*name, actual.clone());
            }
            Ok(())
        }
        (ConstExpr::Value(a), ConstExpr::Value(b)) if a == b => Ok(()),
        (ConstExpr::Inferred(a), ConstExpr::Inferred(b)) if a == b => Ok(()),
        (ConstExpr::Add(l1, r1), ConstExpr::Add(l2, r2))
        | (ConstExpr::Sub(l1, r1), ConstExpr::Sub(l2, r2))
        | (ConstExpr::Mul(l1, r1), ConstExpr::Mul(l2, r2)) => {
            unify_const(l1, l2, subst)?;
            unify_const(r1, r2, subst)
        }
        _ => Err(format!(
            "const mismatch: expected {}, found {}",
            expected, actual
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::{ConstExpr, ConstValue, Ty, TyArg};
    use internment::Intern;

    #[test]
    fn empty() {
        let result = unify_type_args(&[], &[]);
        assert!(result.is_ok());
        let subst = result.unwrap();
        assert!(subst.types.is_empty());
        assert!(subst.consts.is_empty());
    }

    #[test]
    fn assigns_type_var() {
        let expected = vec![TyArg::Type(Box::new(Ty::Opaque(Intern::from_ref("T"))))];
        let actual = vec![TyArg::Type(Box::new(Ty::i64()))];
        let subst = unify_type_args(&expected, &actual).unwrap();
        assert_eq!(subst.types.get(&Intern::from_ref("T")), Some(&Ty::i64()));
    }

    #[test]
    fn type_kind_mismatch() {
        let expected = vec![TyArg::Type(Box::new(Ty::Opaque(Intern::from_ref("T"))))];
        let actual = vec![TyArg::Const(ConstExpr::Value(ConstValue::Int(3)))];
        assert!(unify_type_args(&expected, &actual).is_err());
    }

    #[test]
    fn const_kind_mismatch() {
        let expected = vec![TyArg::Const(ConstExpr::Var(Intern::from_ref("n")))];
        let actual = vec![TyArg::Type(Box::new(Ty::i64()))];
        assert!(unify_type_args(&expected, &actual).is_err());
    }

    #[test]
    fn assigns_const_var() {
        let expected = vec![TyArg::Const(ConstExpr::Var(Intern::from_ref("n")))];
        let actual = vec![TyArg::Const(ConstExpr::Value(ConstValue::Int(8)))];
        let subst = unify_type_args(&expected, &actual).unwrap();
        assert_eq!(
            subst.consts.get(&Intern::from_ref("n")),
            Some(&ConstExpr::Value(ConstValue::Int(8)))
        );
    }

    #[test]
    fn consistent_repeated_var() {
        let t = TyArg::Type(Box::new(Ty::Opaque(Intern::from_ref("T"))));
        let i64 = TyArg::Type(Box::new(Ty::i64()));
        let subst = unify_type_args(&[t.clone(), t], &[i64.clone(), i64]).unwrap();
        assert_eq!(subst.types.len(), 1);
        assert_eq!(subst.types.get(&Intern::from_ref("T")), Some(&Ty::i64()));
    }

    #[test]
    fn inconsistent_type_var() {
        let t = TyArg::Type(Box::new(Ty::Opaque(Intern::from_ref("T"))));
        let subst = unify_type_args(
            &[t.clone(), t],
            &[
                TyArg::Type(Box::new(Ty::i64())),
                TyArg::Type(Box::new(Ty::u8())),
            ],
        );
        assert!(subst.is_err());
    }

    #[test]
    fn inconsistent_const_var() {
        let n = TyArg::Const(ConstExpr::Var(Intern::from_ref("n")));
        let subst = unify_type_args(
            &[n.clone(), n],
            &[
                TyArg::Const(ConstExpr::Value(ConstValue::Int(2))),
                TyArg::Const(ConstExpr::Value(ConstValue::Int(3))),
            ],
        );
        assert!(subst.is_err());
    }

    #[test]
    fn mismatched_length() {
        let expected = vec![TyArg::Type(Box::new(Ty::Opaque(Intern::from_ref("T"))))];
        let actual = vec![];
        assert!(unify_type_args(&expected, &actual).is_err());
    }

    #[test]
    fn mixed_type_and_const() {
        let expected = vec![
            TyArg::Type(Box::new(Ty::Opaque(Intern::from_ref("T")))),
            TyArg::Const(ConstExpr::Var(Intern::from_ref("n"))),
        ];
        let actual = vec![
            TyArg::Type(Box::new(Ty::i64())),
            TyArg::Const(ConstExpr::Value(ConstValue::Int(16))),
        ];
        let subst = unify_type_args(&expected, &actual).unwrap();
        assert_eq!(subst.types.get(&Intern::from_ref("T")), Some(&Ty::i64()));
        assert_eq!(
            subst.consts.get(&Intern::from_ref("n")),
            Some(&ConstExpr::Value(ConstValue::Int(16)))
        );
    }
}
