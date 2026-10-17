use crate::ty::{Ty, reference_semantics_for_type};
use crate::type_registry::TypeRegistry;
use ast::{NormalExpr, TyArg};
use internment::Intern;
use std::collections::HashMap;
use std::fmt::Write;

use crate::normal_expr::{Normalize, normalize_type_constants};
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

fn unify_named_args(
    expected: &[(internment::Intern<String>, TyArg)],
    actual: &[(internment::Intern<String>, TyArg)],
    subst: &mut DepSubst,
    type_registry: Option<&TypeRegistry>,
) -> Result<(), String> {
    if expected.len() != actual.len() {
        return Err(format!(
            "expected {} type arguments, found {}",
            expected.len(),
            actual.len()
        ));
    }
    for ((expected_name, expected), (actual_name, actual)) in expected.iter().zip(actual) {
        if expected_name != actual_name {
            return Err(format!(
                "type argument names do not match: expected `{expected_name}`, found `{actual_name}`"
            ));
        }
        unify_ty_arg(expected, actual, subst, type_registry)?;
    }
    Ok(())
}

fn unify_ty(
    expected: &Ty,
    actual: &Ty,
    subst: &mut DepSubst,
    type_registry: Option<&TypeRegistry>,
) -> Result<(), String> {
    match (expected, actual) {
        (Ty::Proposition(expected), Ty::Proposition(actual)) if expected == actual => Ok(()),
        (
            Ty::Named {
                instance: expected, ..
            },
            Ty::Named {
                instance: actual, ..
            },
        ) if expected == actual => Ok(()),
        (
            Ty::Named {
                instance: expected, ..
            },
            Ty::Named {
                instance: actual, ..
            },
        ) if expected.declaration == actual.declaration => {
            unify_named_args(&expected.arguments, &actual.arguments, subst, type_registry)
        }
        (Ty::Opaque(name), _) => {
            let actual = normalize_type_constants(actual.clone());
            if let Some(existing) = subst.types.get(name) {
                if existing != &actual {
                    return Err(format!(
                        "type parameter `{}` inferred as both {} and {}",
                        name.as_str(),
                        existing.format_for_hover(),
                        actual.format_for_hover()
                    ));
                }
            } else {
                subst.types.insert(*name, actual);
            }
            Ok(())
        }
        (Ty::AnonymousInteger { .. }, Ty::AnonymousInteger { .. })
        | (Ty::Float { .. }, Ty::Float { .. })
        | (Ty::Unit, Ty::Unit) => Ok(()),
        (
            Ty::Record {
                name: n1,
                fields: ef,
                resolved_params: expected_params,
            },
            Ty::Record {
                name: n2,
                fields: af,
                resolved_params: actual_params,
            },
        ) if n1 == n2 => {
            if let (Some(expected), Some(actual)) = (expected_params, actual_params) {
                unify_named_args(expected, actual, subst, type_registry)?;
            }
            if ef.len() != af.len() {
                return Err(format!(
                    "expected {} record fields, found {}",
                    ef.len(),
                    af.len()
                ));
            }
            for ((expected_name, et), (actual_name, at)) in ef.iter().zip(af) {
                if expected_name != actual_name {
                    return Err(format!(
                        "record field names do not match: expected `{expected_name}`, found `{actual_name}`"
                    ));
                }
                unify_ty(et, at, subst, type_registry)?;
            }
            Ok(())
        }
        (
            Ty::Union {
                name: expected_name,
                variants: expected,
                literal_values: expected_literals,
                resolved_params: expected_params,
            },
            Ty::Union {
                name: actual_name,
                variants: actual,
                literal_values: actual_literals,
                resolved_params: actual_params,
            },
        ) if expected_name == actual_name => {
            if let (Some(expected), Some(actual)) = (expected_params, actual_params) {
                unify_named_args(expected, actual, subst, type_registry)?;
            }
            if expected.len() != actual.len() || expected_literals != actual_literals {
                return Err("union variants or literal values do not match".to_string());
            }
            for (expected, actual) in expected.iter().zip(actual) {
                if expected.name != actual.name || expected.kind != actual.kind {
                    return Err("union variant names, kinds or order do not match".to_string());
                }
                if expected.fields.len() != actual.fields.len() {
                    return Err("union variant field counts do not match".to_string());
                }
                for ((expected_name, expected), (actual_name, actual)) in
                    expected.fields.iter().zip(&actual.fields)
                {
                    if expected_name != actual_name {
                        return Err("union variant field names or order do not match".to_string());
                    }
                    unify_ty(expected, actual, subst, type_registry)?;
                }
                match (&expected.result_ty, &actual.result_ty) {
                    (Some(expected), Some(actual)) => {
                        unify_ty(expected, actual, subst, type_registry)?;
                    }
                    (None, None) => {}
                    _ => return Err("union variant result types do not match".to_string()),
                }
            }
            Ok(())
        }
        (Ty::Sequence(e1), Ty::Sequence(e2)) if e1.len() == e2.len() => {
            for (expected, actual) in e1.iter().zip(e2) {
                if expected.name != actual.name {
                    return Err("named result names or order do not match".to_string());
                }
                unify_ty(&expected.ty, &actual.ty, subst, type_registry)?;
            }
            Ok(())
        }
        (Ty::Array { elem: e1, size: s1 }, Ty::Array { elem: e2, size: s2 }) => {
            unify_ty(e1, e2, subst, type_registry)?;
            unify_const(s1, s2, subst)
        }
        (Ty::OwnedStorage { inner: i1 }, Ty::OwnedStorage { inner: i2 })
        | (Ty::OwnedStorage { inner: i1 }, Ty::Ptr { inner: i2 }) => {
            unify_ty(i1, i2, subst, type_registry)
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
            if let (Ty::Ptr { inner: expected }, Ty::OwnedStorage { inner: actual }) =
                (&expected_reference.pointee, &actual_reference.pointee)
            {
                return unify_ty(expected, actual, subst, type_registry);
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
    let inferred = match_const(expected, actual, subst).or_else(|error| {
        match_const(&expected.normalize(), &actual.normalize(), subst).map_err(|_| error)
    })?;
    subst.consts.extend(inferred);
    Ok(())
}

fn match_const(
    expected: &NormalExpr,
    actual: &NormalExpr,
    subst: &DepSubst,
) -> Result<HashMap<Intern<String>, NormalExpr>, String> {
    let mut inferred = HashMap::new();
    let mut pending = vec![(expected, actual)];
    while let Some((expected, actual)) = pending.pop() {
        match (expected, actual) {
            (NormalExpr::Var(name), _) => {
                let actual = actual.normalize();
                if let Some(existing) = inferred.get(name).or_else(|| subst.consts.get(name)) {
                    if existing != &actual {
                        return Err(format!(
                            "const parameter `{}` inferred as both {} and {}",
                            name.as_str(),
                            existing,
                            actual
                        ));
                    }
                } else {
                    inferred.insert(*name, actual);
                }
            }
            (NormalExpr::Value(a), NormalExpr::Value(b)) if a == b => {}
            (NormalExpr::Inferred(a), NormalExpr::Inferred(b)) if a == b => {}
            (
                NormalExpr::TargetQuery {
                    kind: expected_kind,
                    operand: expected_operand,
                },
                NormalExpr::TargetQuery {
                    kind: actual_kind,
                    operand: actual_operand,
                },
            ) if expected_kind == actual_kind && expected_operand == actual_operand => {}
            (NormalExpr::Add(l1, r1), NormalExpr::Add(l2, r2))
            | (NormalExpr::Sub(l1, r1), NormalExpr::Sub(l2, r2))
            | (NormalExpr::Mul(l1, r1), NormalExpr::Mul(l2, r2)) => {
                pending.extend([(r1.as_ref(), r2.as_ref()), (l1.as_ref(), l2.as_ref())]);
            }
            _ => {
                return Err(format!(
                    "const mismatch: expected {}, found {}",
                    expected, actual
                ));
            }
        }
    }
    Ok(inferred)
}
