//! Whether a `when` expression covers all cases of its subject without an `else` arm.

use std::collections::HashSet;

use crate::analysis::pattern::pattern_matches_public;
use crate::ty::Ty;
use crate::typed::TypedWhenArm;
use ast::ConstValue;
use ast::TypeExpr;
use ast::WhenArm;

fn is_patterns_from_arms(arms: &[TypedWhenArm]) -> impl Iterator<Item = &TypeExpr> {
    arms.iter().filter_map(|arm| match arm {
        TypedWhenArm::Is { pattern, .. } => Some(&pattern.value),
        _ => None,
    })
}

fn is_list_tail_catch_all(pattern: &TypeExpr) -> bool {
    match pattern {
        TypeExpr::Nominal(name, _) => name
            .as_str()
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_lowercase()),
        _ => pattern.is_catch_all_pattern(),
    }
}

/// Returns (fixed_prefix_length, tail_is_catch_all) for a cons-chain pattern.
/// `[x, y]` → (2, false),  `[x, ...tail]` → (1, true),  `[x, y, ...tail]` → (2, true).
fn list_cons_depth(pattern: &TypeExpr) -> (usize, bool) {
    match pattern {
        TypeExpr::ListEmpty => (0, false),
        TypeExpr::ListCons { tail, .. } => {
            let (depth, catch_all) = list_cons_depth(&tail.value);
            (depth + 1, catch_all)
        }
        other => (0, is_list_tail_catch_all(other)),
    }
}

fn list_patterns_exhaustive(patterns: &[&TypeExpr]) -> bool {
    let has_empty = patterns.iter().any(|p| matches!(p, TypeExpr::ListEmpty));
    let mut catch_all_cons_min_len = usize::MAX;
    let mut fixed_lengths: HashSet<usize> = HashSet::new();

    for pattern in patterns {
        if let TypeExpr::ListCons { .. } = pattern {
            let (depth, catch_all) = list_cons_depth(pattern);
            if catch_all {
                catch_all_cons_min_len = catch_all_cons_min_len.min(depth);
            } else {
                fixed_lengths.insert(depth);
            }
        }
    }

    // No catch-all cons pattern → infinite list lengths remain uncovered.
    if catch_all_cons_min_len == usize::MAX {
        return false;
    }

    // All lengths >= catch_all_cons_min_len are covered.
    // Check that lengths 0..catch_all_cons_min_len are all covered.
    for len in 0..catch_all_cons_min_len {
        if len == 0 && has_empty {
            continue;
        }
        if len > 0 && fixed_lengths.contains(&len) {
            continue;
        }
        return false; // length `len` is not covered
    }

    true
}

fn union_variants_covered(ty: &Ty, patterns: &[&TypeExpr]) -> bool {
    let Ty::Union { variants, .. } = ty else {
        return false;
    };
    variants.iter().all(|(name, _)| {
        patterns
            .iter()
            .any(|p| p.surface_mangle_name() == name.as_str())
    })
}

/// Check whether all integer values in `[min, max]` are covered by the given patterns.
/// Only literal and in-range patterns are considered — lowercase nominal patterns
/// (value bindings) are not catch-alls for exhaustiveness.
fn integer_range_covered(min: i128, max: i128, patterns: &[&TypeExpr]) -> bool {
    (min..=max).all(|v| {
        let cv = ConstValue::Int(v);
        patterns.iter().any(|p| pattern_matches_public(p, &cv))
    })
}

fn literal_union_values_covered(ty: &Ty, patterns: &[&TypeExpr]) -> bool {
    let Some(values) = ty.union_literal_values() else {
        return false;
    };
    values
        .iter()
        .all(|cv| patterns.iter().any(|p| pattern_matches_public(p, cv)))
}

/// Extension trait providing `Ty::when_subject_exhaustive`.
pub trait TyWhenExt {
    /// `true` when every value of this type is matched by an `is` arm (no `else` needed).
    fn when_subject_exhaustive(&self, arms: &[TypedWhenArm]) -> bool;
}

impl TyWhenExt for Ty {
    fn when_subject_exhaustive(&self, arms: &[TypedWhenArm]) -> bool {
        let patterns: Vec<_> = is_patterns_from_arms(arms).collect();
        if patterns.is_empty() {
            return false;
        }
        if patterns.iter().any(|p| p.is_catch_all_pattern()) {
            return true;
        }
        if list_patterns_exhaustive(&patterns) {
            return true;
        }
        if self.union_literal_values().is_some() {
            literal_union_values_covered(self, &patterns)
        } else {
            match self {
                Ty::Union { .. } => union_variants_covered(self, &patterns),
                Ty::Int {
                    min: Some(min_val),
                    max: Some(max_val),
                    ..
                } => integer_range_covered(*min_val, *max_val, &patterns),
                _ => false,
            }
        }
    }
}

fn is_patterns_from_declare_arms(arms: &[WhenArm]) -> impl Iterator<Item = &TypeExpr> {
    arms.iter().filter_map(|arm| match arm {
        WhenArm::Is { pattern, .. } => Some(&pattern.value),
        _ => None,
    })
}

fn when_subject_exhaustive_declare(subject_ty: &Ty, arms: &[WhenArm]) -> bool {
    let patterns: Vec<_> = is_patterns_from_declare_arms(arms).collect();
    if patterns.is_empty() {
        return false;
    }
    if patterns.iter().any(|p| p.is_catch_all_pattern()) {
        return true;
    }
    if list_patterns_exhaustive(&patterns) {
        return true;
    }
    if subject_ty.union_literal_values().is_some() {
        literal_union_values_covered(subject_ty, &patterns)
    } else {
        match subject_ty {
            Ty::Union { .. } => union_variants_covered(subject_ty, &patterns),
            Ty::Int {
                min: Some(min_val),
                max: Some(max_val),
                ..
            } => integer_range_covered(*min_val, *max_val, &patterns),
            _ => false,
        }
    }
}

/// Condition-based `when` (no subject) is never exhaustive without `else`.
pub fn when_is_exhaustive(subject_ty: Option<&Ty>, arms: &[TypedWhenArm]) -> bool {
    let Some(subject_ty) = subject_ty else {
        return false;
    };
    use TyWhenExt as _;
    subject_ty.when_subject_exhaustive(arms)
}

/// Exhaustiveness for type-level [`WhenArm`] in tag declarations.
pub fn when_declare_is_exhaustive(subject_ty: Option<&Ty>, arms: &[WhenArm]) -> bool {
    let Some(subject_ty) = subject_ty else {
        return false;
    };
    when_subject_exhaustive_declare(subject_ty, arms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::Ty;
    use crate::typed::TypedWhenArm;
    use ast::ConstValue;
    use ast::Literal;
    use ast::span::{SpanId, Spanned};
    use internment::Intern;

    fn intern(s: &str) -> Intern<String> {
        Intern::new(s.to_string())
    }

    fn nominal_pat(name: &str) -> Spanned<TypeExpr> {
        Spanned::new(
            TypeExpr::Nominal(intern(name), SpanId::new(0)),
            SpanId::new(0),
        )
    }

    fn is_arm(name: &str) -> TypedWhenArm {
        TypedWhenArm::Is {
            pattern: Box::new(nominal_pat(name)),
            body: crate::typed::ExprId(0),
            arm_span: ast::span::SubSpan::new(SpanId::new(0)),
        }
    }

    fn list_empty_pat() -> Spanned<TypeExpr> {
        Spanned::new(TypeExpr::ListEmpty, SpanId::new(0))
    }

    fn list_cons_pat(head: TypeExpr, tail: TypeExpr) -> Spanned<TypeExpr> {
        Spanned::new(
            TypeExpr::ListCons {
                head: Box::new(Spanned::new(head, SpanId::new(0))),
                tail: Box::new(Spanned::new(tail, SpanId::new(0))),
            },
            SpanId::new(0),
        )
    }

    fn list_is_arm(pattern: Spanned<TypeExpr>) -> TypedWhenArm {
        TypedWhenArm::Is {
            pattern: Box::new(pattern),
            body: crate::typed::ExprId(0),
            arm_span: ast::span::SubSpan::new(SpanId::new(0)),
        }
    }

    fn pat_ref(arm: &TypedWhenArm) -> &TypeExpr {
        match arm {
            TypedWhenArm::Is { pattern, .. } => &pattern.value,
            _ => unreachable!(),
        }
    }

    #[test]
    fn list_empty_only_not_exhaustive() {
        let arms = [list_is_arm(list_empty_pat())];
        assert!(!list_patterns_exhaustive(&[pat_ref(&arms[0])]));
    }

    #[test]
    fn list_empty_and_cons_catch_all_exhaustive() {
        let arms = [
            list_is_arm(list_empty_pat()),
            list_is_arm(list_cons_pat(
                TypeExpr::Nominal(intern("h"), SpanId::new(0)),
                TypeExpr::Nominal(intern("_"), SpanId::new(0)),
            )),
        ];
        let patterns: Vec<&TypeExpr> = arms.iter().map(pat_ref).collect();
        assert!(list_patterns_exhaustive(&patterns));
    }

    #[test]
    fn list_empty_and_lowercase_rest_cons_exhaustive() {
        let arms = [
            list_is_arm(list_empty_pat()),
            list_is_arm(list_cons_pat(
                TypeExpr::Nominal(intern("h"), SpanId::new(0)),
                TypeExpr::Nominal(intern("rest"), SpanId::new(0)),
            )),
        ];
        let patterns: Vec<&TypeExpr> = arms.iter().map(pat_ref).collect();
        assert!(list_patterns_exhaustive(&patterns));
    }

    #[test]
    fn list_empty_fixed_len_1_and_cons_catch_all_exhaustive() {
        // [] + [h] + [h, t, .._] covers lengths 0, 1, >=2
        let empty = list_empty_pat();
        let fixed_len_1 = list_cons_pat(
            TypeExpr::Nominal(intern("h"), SpanId::new(0)),
            TypeExpr::ListEmpty,
        );
        let cons_catch_all = list_cons_pat(
            TypeExpr::Nominal(intern("h"), SpanId::new(0)),
            list_cons_pat(
                TypeExpr::Nominal(intern("t"), SpanId::new(0)),
                TypeExpr::Nominal(intern("_"), SpanId::new(0)),
            )
            .value,
        );
        let patterns = vec![&empty.value, &fixed_len_1.value, &cons_catch_all.value];
        assert!(list_patterns_exhaustive(&patterns));
    }

    #[test]
    fn list_missing_empty_not_exhaustive() {
        // [h] + [h, t, .._] missing empty
        let fixed_len_1 = list_cons_pat(
            TypeExpr::Nominal(intern("h"), SpanId::new(0)),
            TypeExpr::ListEmpty,
        );
        let cons_catch_all = list_cons_pat(
            TypeExpr::Nominal(intern("h"), SpanId::new(0)),
            list_cons_pat(
                TypeExpr::Nominal(intern("t"), SpanId::new(0)),
                TypeExpr::Nominal(intern("_"), SpanId::new(0)),
            )
            .value,
        );
        let patterns = vec![&fixed_len_1.value, &cons_catch_all.value];
        assert!(!list_patterns_exhaustive(&patterns));
    }

    #[test]
    fn list_missing_intermediate_length_not_exhaustive() {
        // [] + [h, t] + [h, t, z, .._] missing length 1
        let empty = list_empty_pat();
        let fixed_len_2 = list_cons_pat(
            TypeExpr::Nominal(intern("h"), SpanId::new(0)),
            list_cons_pat(
                TypeExpr::Nominal(intern("t"), SpanId::new(0)),
                TypeExpr::ListEmpty,
            )
            .value,
        );
        let cons_catch_all = list_cons_pat(
            TypeExpr::Nominal(intern("h"), SpanId::new(0)),
            list_cons_pat(
                TypeExpr::Nominal(intern("t"), SpanId::new(0)),
                list_cons_pat(
                    TypeExpr::Nominal(intern("z"), SpanId::new(0)),
                    TypeExpr::Nominal(intern("_"), SpanId::new(0)),
                )
                .value,
            )
            .value,
        );
        let patterns = vec![&empty.value, &fixed_len_2.value, &cons_catch_all.value];
        assert!(!list_patterns_exhaustive(&patterns));
    }

    #[test]
    fn bool_union_two_arms_exhaustive() {
        let ty = Ty::union_named(
            intern("Bool"),
            vec![(intern("True"), vec![]), (intern("False"), vec![])],
        );
        let arms = vec![is_arm("True"), is_arm("False")];
        assert!(ty.when_subject_exhaustive(&arms));
    }

    #[test]
    fn bool_union_one_arm_not_exhaustive() {
        let ty = Ty::union_named(
            intern("Bool"),
            vec![(intern("True"), vec![]), (intern("False"), vec![])],
        );
        let arms = vec![is_arm("True")];
        assert!(!ty.when_subject_exhaustive(&arms));
    }

    #[test]
    fn wildcard_arm_exhaustive() {
        let ty = Ty::Opaque(intern("Type"));
        let arms = vec![is_arm("_")];
        assert!(ty.when_subject_exhaustive(&arms));
    }

    #[test]
    fn lowercase_pattern_is_not_exhaustive() {
        let ty = Ty::Opaque(intern("Type"));
        let arms = vec![is_arm("value")];
        assert!(!ty.when_subject_exhaustive(&arms));
    }

    #[test]
    fn const_union_literals_exhaustive() {
        let ty = Ty::union_of_literals(
            intern("Arch"),
            vec![
                ConstValue::String("x86_64".to_string()),
                ConstValue::String("wasm32".to_string()),
            ],
        );
        let arms = vec![
            TypedWhenArm::Is {
                pattern: Box::new(Spanned::new(
                    TypeExpr::Literal(Literal::String("x86_64".to_string()), SpanId::new(0)),
                    SpanId::new(0),
                )),
                body: crate::typed::ExprId(0),
                arm_span: ast::span::SubSpan::new(SpanId::new(0)),
            },
            TypedWhenArm::Is {
                pattern: Box::new(Spanned::new(
                    TypeExpr::Literal(Literal::String("wasm32".to_string()), SpanId::new(0)),
                    SpanId::new(0),
                )),
                body: crate::typed::ExprId(0),
                arm_span: ast::span::SubSpan::new(SpanId::new(0)),
            },
        ];
        assert!(ty.when_subject_exhaustive(&arms));
    }

    #[test]
    fn no_subject_never_exhaustive() {
        let arms = vec![is_arm("True")];
        assert!(!when_is_exhaustive(None, &arms));
    }
}
