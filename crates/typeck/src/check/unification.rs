use std::collections::HashMap;

use internment::Intern;

use crate::ty::Ty;

/// One-way structural unification check between an actual type and an expected
/// type, with type-variable bindings collected in `bindings`.
pub fn ty_unifies_with(
    actual: &Ty,
    expected: &Ty,
    bindings: &mut HashMap<Intern<String>, Ty>,
) -> bool {
    if tys_equivalent(actual, expected) {
        return true;
    }
    match (actual, expected) {
        (_, Ty::Opaque(name)) => {
            let is_unbound = match bindings.get(name) {
                None => true,
                Some(Ty::Opaque(prev)) if prev == name => true,
                _ => false,
            };
            if is_unbound {
                bindings.insert(*name, strip_literal(actual));
                return true;
            }
            bindings
                .get(name)
                .map(|prev| tys_equivalent(prev, actual))
                .unwrap_or(false)
        }
        (Ty::Opaque(_), _) => true,
        (Ty::Tuple(elems), Ty::Record { fields, .. }) => {
            if elems.len() != fields.len() {
                return false;
            }
            elems
                .iter()
                .zip(fields.iter())
                .all(|(e, (_, f))| ty_unifies_with(e, f, bindings))
        }
        (Ty::Record { fields, .. }, Ty::Tuple(elems)) => {
            if fields.len() != elems.len() {
                return false;
            }
            fields
                .iter()
                .zip(elems.iter())
                .all(|((_, f), e)| ty_unifies_with(f, e, bindings))
        }
        (Ty::Tuple(a), Ty::Tuple(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| ty_unifies_with(x, y, bindings))
        }
        (
            Ty::Record {
                fields: af,
                name: an,
            },
            Ty::Record {
                fields: bf,
                name: bn,
            },
        ) => {
            an == bn
                && af.len() == bf.len()
                && af
                    .iter()
                    .zip(bf.iter())
                    .all(|((_, a), (_, b))| ty_unifies_with(a, b, bindings))
        }
        (Ty::Ptr { inner: a }, Ty::Ref { inner: b })
        | (Ty::Ref { inner: a }, Ty::Ptr { inner: b })
        | (Ty::Ptr { inner: a }, Ty::Ptr { inner: b })
        | (Ty::Ref { inner: a }, Ty::Ref { inner: b }) => ty_unifies_with(a, b, bindings),
        (
            Ty::ConstUnion {
                values: av,
                base: ab,
                ..
            },
            Ty::ConstUnion {
                values: bv,
                base: bb,
                ..
            },
        ) => {
            let base_match = match (ab.as_ref(), bb.as_ref()) {
                (Ty::Int { .. }, Ty::Int { .. }) => true,
                (Ty::Float { .. }, Ty::Float { .. }) => true,
                _ => tys_equivalent(ab, bb),
            };
            base_match && av.iter().all(|v| bv.contains(v))
        }
        (Ty::ConstUnion { .. }, _) | (_, Ty::ConstUnion { .. }) => false,
        _ => false,
    }
}

/// Structural type equivalence ignoring literal `value` fields on Int/Float.
pub(crate) fn tys_equivalent(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (
            Ty::Int {
                width: aw,
                signed: as_,
                ..
            },
            Ty::Int {
                width: bw,
                signed: bs,
                ..
            },
        ) => aw == bw && as_ == bs,
        (Ty::Float { .. }, Ty::Float { .. }) => true,
        (Ty::Tuple(av), Ty::Tuple(bv)) => {
            av.len() == bv.len() && av.iter().zip(bv.iter()).all(|(x, y)| tys_equivalent(x, y))
        }
        (
            Ty::Record {
                name: an,
                fields: af,
            },
            Ty::Record {
                name: bn,
                fields: bf,
            },
        ) => {
            an == bn
                && af.len() == bf.len()
                && af
                    .iter()
                    .zip(bf.iter())
                    .all(|((_, x), (_, y))| tys_equivalent(x, y))
        }
        (Ty::Ptr { inner: a }, Ty::Ptr { inner: b })
        | (Ty::Ref { inner: a }, Ty::Ref { inner: b }) => tys_equivalent(a, b),
        _ => a == b,
    }
}

fn strip_literal(ty: &Ty) -> Ty {
    match ty {
        Ty::Int { width, signed, .. } => Ty::Int {
            width: *width,
            signed: *signed,
            value: None,
        },
        Ty::Float { .. } => Ty::Float { value: None },
        _ => ty.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::ConstValue;

    fn int_ty(width: u8) -> Ty {
        Ty::Int { width, signed: true, value: None }
    }

    fn const_union(name: &str, values: Vec<ConstValue>) -> Ty {
        Ty::ConstUnion {
            name: Intern::from_ref(name),
            values,
            base: Box::new(int_ty(64)),
        }
    }

    #[test]
    fn identical_types_unify() {
        let mut bindings = HashMap::new();
        assert!(ty_unifies_with(&int_ty(64), &int_ty(64), &mut bindings));
    }

    #[test]
    fn different_widths_do_not_unify() {
        let mut bindings = HashMap::new();
        assert!(!ty_unifies_with(&int_ty(32), &int_ty(64), &mut bindings));
    }

    #[test]
    fn opaque_binds_actual_type() {
        let mut bindings = HashMap::new();
        let opaque = Ty::Opaque(Intern::from_ref("T"));
        assert!(ty_unifies_with(&int_ty(64), &opaque, &mut bindings));
        assert_eq!(bindings.get(&Intern::from_ref("T")), Some(&int_ty(64)));
    }

    #[test]
    fn opaque_subsequent_use_must_match() {
        let mut bindings = HashMap::new();
        let opaque = Ty::Opaque(Intern::from_ref("T"));
        assert!(ty_unifies_with(&int_ty(64), &opaque, &mut bindings));
        assert!(!ty_unifies_with(&int_ty(32), &opaque, &mut bindings));
    }

    #[test]
    fn const_union_subset_unifies() {
        let mut bindings = HashMap::new();
        let a = const_union("LogLevel", vec![
            ConstValue::Int(1),
            ConstValue::Int(2),
        ]);
        let b = const_union("LogLevel", vec![
            ConstValue::Int(1),
            ConstValue::Int(2),
            ConstValue::Int(3),
        ]);
        assert!(ty_unifies_with(&a, &b, &mut bindings));
    }
}
