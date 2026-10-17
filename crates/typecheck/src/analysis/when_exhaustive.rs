//! Whether a `when` expression covers all cases of its subject without an `else` arm.

use std::collections::HashSet;

use crate::analysis::pattern::pattern_matches_public;
use crate::ty::Ty;
use crate::typed::TypedWhenArm;
use ast::Pattern;
use ast::WhenArm;

pub(crate) fn is_patterns_from_arms(arms: &[TypedWhenArm]) -> impl Iterator<Item = &Pattern> {
    let mut patterns = Vec::new();
    for arm in arms {
        match arm {
            TypedWhenArm::Is { pattern, .. } => patterns.push(&pattern.value),
            TypedWhenArm::Cond { coverage, .. } => patterns.extend(coverage),
            _ => {}
        }
    }
    patterns.into_iter()
}

/// Returns (fixed_prefix_length, tail_is_catch_all) for a cons-chain pattern.
/// `[x, y]` → (2, false),  `[x, ...tail]` → (1, true),  `[x, y, ...tail]` → (2, true).
fn list_cons_depth(pattern: &Pattern) -> (usize, bool) {
    pattern
        .list_cons_shape()
        .unwrap_or((0, pattern.is_list_tail_catch_all()))
}

fn list_patterns_exhaustive(patterns: &[&Pattern]) -> bool {
    let has_empty = patterns.iter().any(|p| p.is_list_empty());
    let mut catch_all_cons_min_len = usize::MAX;
    let mut fixed_lengths: HashSet<usize> = HashSet::new();

    for pattern in patterns {
        if pattern.is_list_cons() && list_elements_irrefutable(pattern) {
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

fn list_elements_irrefutable(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::ListEmpty => true,
        Pattern::ListCons { head, tail } => {
            head.value.is_catch_all_pattern() && list_elements_irrefutable(&tail.value)
        }
        _ => pattern.is_catch_all_pattern(),
    }
}

fn list_pattern_covers_length(pattern: &Pattern, length: usize) -> bool {
    match pattern {
        Pattern::ListEmpty => length == 0,
        Pattern::ListCons { head, tail } => {
            length > 0
                && head.value.is_catch_all_pattern()
                && list_pattern_covers_length(&tail.value, length - 1)
        }
        _ => pattern.is_catch_all_pattern(),
    }
}

pub(super) fn parameter_covers_type(
    name: &internment::Intern<String>,
    kind: &ast::parameter::ParameterKind,
    ty: &Ty,
    registry: Option<&crate::TypeRegistry>,
) -> bool {
    let pattern = match kind {
        ast::parameter::ParameterKind::Proposition(_) | ast::parameter::ParameterKind::Generic => {
            Pattern::Nominal(*name, ast::SpanId::INVALID)
        }
        ast::parameter::ParameterKind::Tagged(expr)
        | ast::parameter::ParameterKind::ValueParam { ty: expr }
        | ast::parameter::ParameterKind::Inferred { ty: expr } => {
            Pattern::from_expr(expr.value.clone())
        }
        ast::parameter::ParameterKind::Default(expr) => match &expr.value {
            ast::expr::Expr::Lit(_)
            | ast::expr::Expr::AnonymousTag(_)
            | ast::expr::Expr::TagCall(_) => Pattern::from_expr(expr.value.clone()),
            ast::expr::Expr::FnCall(call)
                if call.args.as_ref().is_none_or(Vec::is_empty)
                    && call.path.value.segments.is_empty() =>
            {
                Pattern::from_expr(expr.value.clone())
            }
            _ => return false,
        },
    };
    if matches!(kind, ast::parameter::ParameterKind::Default(_))
        && let Pattern::Nominal(name, _) = &pattern
    {
        let matches_tag = |value: &ast::ConstValue| matches!(value, ast::ConstValue::Tag { name: value_name, .. } if value_name == name);
        return match ty {
            Ty::Literal(value) => matches_tag(value),
            Ty::Union {
                literal_values: Some(values),
                ..
            } => values.iter().all(matches_tag),
            Ty::Union { variants, .. } => variants.iter().all(|variant| variant.name == *name),
            _ => false,
        };
    }
    patterns_exhaustive_with_registry(ty, &[&pattern], registry)
}

pub(crate) fn pattern_covers_union_variant(
    pattern: &Pattern,
    variant: &ast::ty::UnionVariant,
    registry: Option<&crate::TypeRegistry>,
) -> bool {
    if pattern.surface_mangle_name() != variant.name.as_str() {
        return false;
    }
    let Pattern::Generic {
        params,
        param_spans,
        ..
    } = pattern
    else {
        return matches!(pattern, Pattern::Nominal(..) | Pattern::Qualified(..));
    };
    if param_spans.len() != variant.fields.len() {
        return false;
    }
    param_spans
        .iter()
        .zip(&variant.fields)
        .all(|((name, _), (_, ty))| {
            let Some((_, kind)) = params.iter().find(|(parameter, _)| parameter == name) else {
                return false;
            };
            parameter_covers_type(name, kind, ty, registry)
        })
}

fn union_variants_covered(
    ty: &Ty,
    patterns: &[&Pattern],
    registry: Option<&crate::TypeRegistry>,
) -> bool {
    let Ty::Union { variants, .. } = ty else {
        return false;
    };
    variants.iter().all(|variant| {
        super::constructor_coverage::patterns_cover_union_variant(patterns, variant, registry)
    })
}

/// Check whether all integer values in `[min, max]` are covered by the given patterns.
fn integer_range_covered(min: i128, max: i128, patterns: &[&Pattern]) -> bool {
    let mut intervals: Vec<_> = patterns
        .iter()
        .filter_map(|pattern| match pattern {
            Pattern::InRange {
                bounds: ast::InRangeBounds::Literal(low, high),
                ..
            } => Some((*low, *high)),
            Pattern::Literal(ast::Literal::Int(value), _) => Some((*value, *value)),
            _ => None,
        })
        .collect();
    intervals.sort_unstable();
    let mut next = i256::I256::from(min);
    let max = i256::I256::from(max);
    for (low, high) in intervals {
        if next > max {
            return true;
        }
        if low > next {
            return false;
        }
        if high >= max {
            return true;
        }
        if high >= next {
            next = high + i256::I256::from(1);
        }
    }
    next > max
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
    patterns_exhaustive_with_registry(subject_ty, patterns, None)
}

fn patterns_exhaustive_with_registry(
    subject_ty: &Ty,
    patterns: &[&Pattern],
    registry: Option<&crate::TypeRegistry>,
) -> bool {
    let resolved = registry.map(|registry| registry.resolved_definition_for_type(subject_ty));
    let subject_ty = resolved.as_ref().unwrap_or(subject_ty);
    if patterns.is_empty() {
        return false;
    }
    if patterns.iter().any(|p| p.is_catch_all_pattern()) {
        return true;
    }
    match subject_ty {
        Ty::Ref { inner, .. } => {
            return patterns_exhaustive_with_registry(inner, patterns, registry);
        }
        Ty::Literal(value) => {
            return patterns
                .iter()
                .any(|pattern| pattern_matches_public(pattern, value));
        }
        Ty::ResultFamily { alternatives, .. } => {
            return alternatives.iter().all(|alternative| {
                let variant = ast::ty::UnionVariant::new(alternative.label, Vec::new());
                super::constructor_coverage::patterns_cover_union_variant(
                    patterns, &variant, registry,
                )
            });
        }
        Ty::Array { size, .. } => {
            return size
                .as_const_int()
                .and_then(|length| usize::try_from(length).ok())
                .is_some_and(|length| {
                    patterns
                        .iter()
                        .any(|pattern| list_pattern_covers_length(pattern, length))
                });
        }
        Ty::Sequence(fields) => {
            return patterns
                .iter()
                .any(|pattern| list_pattern_covers_length(pattern, fields.len()));
        }
        _ => {}
    }
    if list_patterns_exhaustive(patterns) {
        return true;
    }
    if subject_ty.union_literal_values().is_some() {
        literal_union_values_covered(subject_ty, patterns)
    } else if matches!(subject_ty, Ty::Union { .. }) {
        union_variants_covered(subject_ty, patterns, registry)
    } else if let Some(bounds) = subject_ty.int_bounds() {
        integer_range_covered(bounds.min, bounds.max, patterns)
    } else {
        false
    }
}

/// Condition-based `when` (no subject) is never exhaustive without `else`.
pub fn when_is_exhaustive(
    subject_ty: Option<&Ty>,
    arms: &[TypedWhenArm],
    registry: &crate::TypeRegistry,
) -> bool {
    let Some(subject_ty) = subject_ty else {
        return false;
    };
    let patterns: Vec<_> = is_patterns_from_arms(arms).collect();
    patterns_exhaustive_with_registry(subject_ty, &patterns, Some(registry))
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

    fn constructor(parameter: &str, kind: ast::parameter::ParameterKind) -> Pattern {
        let parameter = internment::Intern::from_ref(parameter);
        Pattern::Generic {
            name: internment::Intern::from_ref("Some"),
            params: vec![(parameter, kind)],
            param_spans: vec![(parameter, ast::SpanId::INVALID)],
            span: ast::SpanId::INVALID,
        }
    }

    fn optional_byte() -> Ty {
        Ty::Union {
            name: internment::Intern::from_ref("Optional"),
            variants: vec![
                ast::ty::UnionVariant::new(
                    internment::Intern::from_ref("Some"),
                    vec![(
                        internment::Intern::from_ref("value"),
                        Box::new(Ty::bounded_int(0.into(), 255.into())),
                    )],
                ),
                ast::ty::UnionVariant::new(internment::Intern::from_ref("None"), vec![]),
            ],
            literal_values: None,
            resolved_params: None,
        }
    }

    #[test]
    fn default_tag_payload_matches_a_value_instead_of_binding_a_name() {
        let name = internment::Intern::from_ref("value");
        let value = |name: &str| ast::ConstValue::Tag {
            name: internment::Intern::from_ref(name),
            args: vec![].into(),
            qual_path: None,
        };
        let some = constructor(
            "tag",
            ast::parameter::ParameterKind::Default(Box::new(ast::expr::Typed::new(
                ast::expr::Expr::AnonymousTag(internment::Intern::from_ref("first")),
                ast::TyState::Infer,
                ast::SpanId::INVALID,
            ))),
        );
        let variant = |ty| {
            ast::ty::UnionVariant::new(
                internment::Intern::from_ref("Some"),
                vec![(name, Box::new(ty))],
            )
        };
        assert!(!pattern_covers_union_variant(
            &some,
            &variant(Ty::union_of_literals(
                name,
                vec![value("first"), value("second")]
            )),
            None
        ));
        assert!(pattern_covers_union_variant(
            &some,
            &variant(Ty::Literal(value("first"))),
            None
        ));
        assert!(!pattern_covers_union_variant(
            &some,
            &variant(Ty::Literal(value("second"))),
            None
        ));
    }

    #[test]
    fn constructor_wildcard_does_not_cover_other_variants() {
        let some = constructor("_", ast::parameter::ParameterKind::Generic);
        assert!(!some.is_catch_all_pattern());
        assert!(!patterns_exhaustive(&optional_byte(), &[&some]));
    }

    #[test]
    fn constrained_constructor_does_not_cover_every_payload() {
        let some = constructor(
            "Zero",
            ast::parameter::ParameterKind::Default(Box::new(ast::expr::Typed::new(
                ast::expr::Expr::Lit(ast::Literal::Int(0.into())),
                ast::TyState::Infer,
                ast::SpanId::INVALID,
            ))),
        );
        let none = Pattern::Nominal(internment::Intern::from_ref("None"), ast::SpanId::INVALID);
        assert!(!patterns_exhaustive(&optional_byte(), &[&some, &none]));
    }

    #[test]
    fn irrefutable_constructor_payloads_cover_the_union() {
        let none = Pattern::Nominal(internment::Intern::from_ref("None"), ast::SpanId::INVALID);
        for parameter in ["_", "value"] {
            let some = constructor(parameter, ast::parameter::ParameterKind::Generic);
            assert!(patterns_exhaustive(&optional_byte(), &[&some, &none]));
        }
    }

    #[test]
    fn constructor_payload_arity_must_match_for_coverage() {
        let mut some = constructor("value", ast::parameter::ParameterKind::Generic);
        if let Pattern::Generic {
            params,
            param_spans,
            ..
        } = &mut some
        {
            params.clear();
            param_spans.clear();
        }
        let none = Pattern::Nominal(internment::Intern::from_ref("None"), ast::SpanId::INVALID);
        assert!(!patterns_exhaustive(&optional_byte(), &[&some, &none]));
    }

    #[test]
    fn integer_coverage_handles_full_width_ranges_and_single_value_gaps() {
        let interval = |low: i128, high: i128| Pattern::InRange {
            bounds: ast::InRangeBounds::Literal(low.into(), high.into()),
            span: ast::SpanId::INVALID,
        };
        let lower = interval(i128::MIN, -1);
        let upper = interval(0, i128::MAX);
        assert!(integer_range_covered(
            i128::MIN,
            i128::MAX,
            &[&upper, &lower]
        ));
        let gap = interval(1, i128::MAX);
        assert!(!integer_range_covered(
            i128::MIN,
            i128::MAX,
            &[&lower, &gap]
        ));
        let zero = Pattern::Literal(ast::Literal::Int(0.into()), ast::SpanId::INVALID);
        assert!(integer_range_covered(
            i128::MIN,
            i128::MAX,
            &[&lower, &gap, &zero]
        ));
        assert!(!integer_range_covered(0, 0, &[]));
    }
}
