//! Whether a `when` expression covers all cases of its subject without an `else` arm.

use std::collections::HashSet;

use crate::analysis::pattern::pattern_matches_public;
use crate::ty::Ty;
use crate::typed::TypedWhenArm;
use ast::ConstValue;
use ast::Pattern;
use ast::WhenArm;

fn is_patterns_from_arms(arms: &[TypedWhenArm]) -> impl Iterator<Item = &Pattern> {
    arms.iter().filter_map(|arm| match arm {
        TypedWhenArm::Is { pattern, .. } => Some(&pattern.value),
        _ => None,
    })
}

fn is_list_tail_catch_all(pattern: &Pattern) -> bool {
    match pattern.nominal_name() {
        Some(name) => name
            .as_str()
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_lowercase()),
        None => pattern.is_catch_all_pattern(),
    }
}

/// Returns (fixed_prefix_length, tail_is_catch_all) for a cons-chain pattern.
/// `[x, y]` → (2, false),  `[x, ...tail]` → (1, true),  `[x, y, ...tail]` → (2, true).
fn list_cons_depth(pattern: &Pattern) -> (usize, bool) {
    pattern
        .list_cons_shape()
        .unwrap_or((0, is_list_tail_catch_all(pattern)))
}

fn list_patterns_exhaustive(patterns: &[&Pattern]) -> bool {
    let has_empty = patterns.iter().any(|p| p.is_list_empty());
    let mut catch_all_cons_min_len = usize::MAX;
    let mut fixed_lengths: HashSet<usize> = HashSet::new();

    for pattern in patterns {
        if pattern.is_list_cons() {
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

fn union_variants_covered(ty: &Ty, patterns: &[&Pattern]) -> bool {
    let Ty::Union { variants, .. } = ty else {
        return false;
    };
    variants.iter().all(|v| {
        patterns
            .iter()
            .any(|p| p.surface_mangle_name() == v.name.as_str())
    })
}

/// Check whether all integer values in `[min, max]` are covered by the given patterns.
fn integer_range_covered(min: i128, max: i128, patterns: &[&Pattern]) -> bool {
    (min..=max).all(|v| {
        let cv = ConstValue::Int(v.into());
        patterns.iter().any(|p| pattern_matches_public(p, &cv))
    })
}

fn literal_union_values_covered(ty: &Ty, patterns: &[&Pattern]) -> bool {
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
        patterns_exhaustive(self, &patterns)
    }
}

fn is_patterns_from_declare_arms(arms: &[WhenArm]) -> impl Iterator<Item = &Pattern> {
    arms.iter().filter_map(|arm| match arm {
        WhenArm::Is { pattern, .. } => Some(&pattern.value),
        _ => None,
    })
}

fn when_subject_exhaustive_declare(subject_ty: &Ty, arms: &[WhenArm]) -> bool {
    let patterns: Vec<_> = is_patterns_from_declare_arms(arms).collect();
    patterns_exhaustive(subject_ty, &patterns)
}

fn patterns_exhaustive(subject_ty: &Ty, patterns: &[&Pattern]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    if patterns.iter().any(|p| p.is_catch_all_pattern()) {
        return true;
    }
    if list_patterns_exhaustive(patterns) {
        return true;
    }
    if subject_ty.union_literal_values().is_some() {
        literal_union_values_covered(subject_ty, patterns)
    } else if matches!(subject_ty, Ty::Union { .. }) {
        union_variants_covered(subject_ty, patterns)
    } else if let Some(bounds) = subject_ty.int_bounds() {
        integer_range_covered(bounds.min, bounds.max, patterns)
    } else {
        false
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
#[path = "../../tests/when_exhaustive_tests.rs"]
mod tests;
