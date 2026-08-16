use crate::ty::{Ty, reference_semantics_for_type};
use crate::type_registry::TypeRegistry;
use ast::{NormalExpr, TyArg};
use std::fmt::Write;

use crate::subst::DepSubst;

/// Walk two `TyArg` lists pairwise, unifying each pair and building a `DepSubst`.
pub fn unify_type_args(expected: &[TyArg], actual: &[TyArg]) -> Result<DepSubst, String> {
    unify_type_args_with_registry(expected, actual, None)
}

pub fn unify_type_args_with_registry(
    expected: &[TyArg],
    actual: &[TyArg],
    type_registry: Option<&TypeRegistry>,
) -> Result<DepSubst, String> {
    if expected.len() != actual.len() {
        return Err(format!(
            "expected {} type arguments, found {}",
            expected.len(),
            actual.len()
        ));
    }
    let mut subst = DepSubst::new();
    for (exp, act) in expected.iter().zip(actual.iter()) {
        unify_ty_arg(exp, act, &mut subst, type_registry)?;
    }
    Ok(subst)
}

fn unify_ty_arg(
    expected: &TyArg,
    actual: &TyArg,
    subst: &mut DepSubst,
    type_registry: Option<&TypeRegistry>,
) -> Result<(), String> {
    match (expected, actual) {
        (TyArg::Type(exp), TyArg::Type(act)) => unify_ty(exp, act, subst, type_registry),
        (TyArg::Const(exp), TyArg::Const(act)) => unify_const(exp, act, subst),
        (TyArg::Type(_), _) => Err("expected type argument, found const".to_string()),
        (TyArg::Const(_), _) => Err("expected const argument, found type".to_string()),
    }
}

fn unify_ty(
    expected: &Ty,
    actual: &Ty,
    subst: &mut DepSubst,
    type_registry: Option<&TypeRegistry>,
) -> Result<(), String> {
    match (expected, actual) {
        (
            Ty::Named {
                instance: expected, ..
            },
            Ty::Named {
                instance: actual, ..
            },
        ) if expected == actual => Ok(()),
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
        (Ty::AnonymousInteger { .. }, Ty::AnonymousInteger { .. })
        | (Ty::Float { .. }, Ty::Float { .. })
        | (Ty::Unit, Ty::Unit) => Ok(()),
        (Ty::Record { name: n1, .. }, Ty::Record { name: n2, .. }) if n1 == n2 => {
            if let (Ty::Record { fields: ef, .. }, Ty::Record { fields: af, .. }) =
                (expected, actual)
                && ef.len() == af.len()
            {
                for ((_, et), (_, at)) in ef.iter().zip(af.iter()) {
                    unify_ty(et, at, subst, type_registry)?;
                }
            }
            Ok(())
        }
        (Ty::Union { name: n1, .. }, Ty::Union { name: n2, .. }) if n1 == n2 => Ok(()),
        (Ty::Tuple(e1), Ty::Tuple(e2)) if e1.len() == e2.len() => {
            for (et, at) in e1.iter().zip(e2.iter()) {
                unify_ty(et, at, subst, type_registry)?;
            }
            Ok(())
        }
        (Ty::Array { elem: e1, size: s1 }, Ty::Array { elem: e2, size: s2 }) => {
            unify_ty(e1, e2, subst, type_registry)?;
            unify_const(s1, s2, subst)
        }
        (Ty::Ptr { inner: i1 }, Ty::Ptr { inner: i2 }) => unify_ty(i1, i2, subst, type_registry),
        (
            Ty::Address {
                pointee: i1,
                address_space: a1,
            },
            Ty::Address {
                pointee: i2,
                address_space: a2,
            },
        ) if a1 == a2 => unify_ty(i1, i2, subst, type_registry),
        (expected, actual)
            if let (Some(expected_reference), Some(actual_reference)) = (
                reference_semantics_for_type(expected, type_registry),
                reference_semantics_for_type(actual, type_registry),
            ) =>
        {
            if expected_reference.permission != actual_reference.permission {
                let mut msg = String::new();
                let _ = write!(
                    msg,
                    "type mismatch: expected {}, found {}",
                    expected.format_for_hover(),
                    actual.format_for_hover()
                );
                return Err(msg);
            }
            unify_ty(
                &expected_reference.pointee,
                &actual_reference.pointee,
                subst,
                type_registry,
            )
        }
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
    expected: &NormalExpr,
    actual: &NormalExpr,
    subst: &mut DepSubst,
) -> Result<(), String> {
    match (expected, actual) {
        (NormalExpr::Var(name), _) => {
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
        (NormalExpr::Value(a), NormalExpr::Value(b)) if a == b => Ok(()),
        (NormalExpr::Inferred(a), NormalExpr::Inferred(b)) if a == b => Ok(()),
        (NormalExpr::Add(l1, r1), NormalExpr::Add(l2, r2))
        | (NormalExpr::Sub(l1, r1), NormalExpr::Sub(l2, r2))
        | (NormalExpr::Mul(l1, r1), NormalExpr::Mul(l2, r2)) => {
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
#[path = "../../tests/unify_tests.rs"]
mod tests;
