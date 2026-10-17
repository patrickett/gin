//! Pattern matching — check whether a [`Pattern`] matches a [`ConstValue`].
//!
//! Used by compile-time expression evaluation (when-arm matching, tag-declare
//! simplification) and by trait/reflect machinery.

use std::collections::HashMap;

use internment::Intern;

use crate::ty::Ty;
use crate::typed::TagId;
use ast::InRangeBounds;
use ast::WhenArm;
use ast::expr::{Expr, Literal, Typed};
use ast::parameter::ParameterKind;
use ast::type_decl::TypeNameExt;
use ast::{ConstValue, HashFloat, Pattern};

/// Public entry: check whether a [`Pattern`] matches a [`ConstValue`].
pub fn pattern_matches_public(pattern: &Pattern, cv: &ConstValue) -> bool {
    pattern_matches(pattern, cv)
}

pub fn pattern_matches_with_tag_types(
    pattern: &Pattern,
    cv: &ConstValue,
    tag_types: &HashMap<TagId, Ty>,
    registry: &crate::TypeRegistry,
) -> bool {
    let validity = match pattern {
        Pattern::InRange {
            bounds: InRangeBounds::Tag(name),
            ..
        } => tag_types
            .get(&TagId(*name))
            .and_then(|ty| registry.integer_validity_for_type(ty)),
        _ => None,
    };
    pattern_matches_with_integer_validity(pattern, cv, validity).unwrap_or(false)
}

pub(crate) fn pattern_matches_with_integer_validity(
    pattern: &Pattern,
    value: &ConstValue,
    validity: Option<&ast::integer::IntegerValidity>,
) -> Option<bool> {
    match (pattern, value) {
        (
            Pattern::InRange {
                bounds: InRangeBounds::Tag(_),
                ..
            },
            ConstValue::Int(value),
        ) => match validity?.domain().membership(*value) {
            ast::integer::ContainmentProof::Proven => Some(true),
            ast::integer::ContainmentProof::Disproven => Some(false),
            ast::integer::ContainmentProof::Unknown => None,
        },
        (
            Pattern::InRange {
                bounds: InRangeBounds::LiteralToTag(..),
                ..
            },
            _,
        ) => None,
        _ => Some(pattern_matches(pattern, value)),
    }
}

/// Check whether a when-arm's pattern matches a subject value.
/// Returns `Some(body)` when a matching arm is found.
///
/// Used by intrinsic folding and tag-declare simplification where we need
/// to select a when arm at compile time but don't need full evaluation.
pub(crate) fn find_matching_when_body<'a>(
    arms: &'a [WhenArm],
    subject: &Expr,
    cv: &ConstValue,
) -> Option<&'a Typed<Expr>> {
    for arm in arms {
        match arm {
            WhenArm::Is { pattern, body, .. } => {
                if pattern_matches_with_integer_validity(&pattern.value, cv, None)? {
                    return Some(body);
                }
            }
            WhenArm::Else(body, _) => return Some(body),
            WhenArm::Cond {
                condition, body, ..
            } => {
                if condition_matches_subject(condition, subject, cv)? {
                    return Some(body);
                }
            }
        }
    }
    None
}

fn condition_matches_subject(
    condition: &ast::Condition,
    subject: &Expr,
    value: &ConstValue,
) -> Option<bool> {
    match condition {
        ast::Condition::Is {
            subject: tested,
            pattern,
        } if tested.value == *subject => {
            pattern_matches_with_integer_validity(&pattern.value, value, None)
        }
        ast::Condition::Or(left, right) => {
            if condition_matches_subject(left, subject, value)? {
                Some(true)
            } else {
                condition_matches_subject(right, subject, value)
            }
        }
        ast::Condition::And(left, right) => {
            if condition_matches_subject(left, subject, value)? {
                condition_matches_subject(right, subject, value)
            } else {
                Some(false)
            }
        }
        _ => None,
    }
}

/// Core pattern matching — does a `Pattern` match a `ConstValue`?
pub(crate) fn pattern_matches(pattern: &Pattern, cv: &ConstValue) -> bool {
    match (pattern, cv) {
        (Pattern::Literal(Literal::String(s), _), ConstValue::Text(cv_s)) => s == cv_s,
        (Pattern::Literal(Literal::Int(n), _), ConstValue::Int(cv_n)) => *n == *cv_n,
        (Pattern::Literal(Literal::Float(HashFloat(f)), _), ConstValue::Float(HashFloat(cv_f))) => {
            f.to_bits() == cv_f.to_bits()
        }
        (
            Pattern::InRange {
                bounds: InRangeBounds::Literal(min, max),
                ..
            },
            ConstValue::Int(n),
        ) => *n >= *min && *n <= *max,
        (Pattern::Nominal(name, _), _)
            if name.as_str().starts_with_lowercase() || name.as_str() == "_" =>
        {
            true
        }
        (Pattern::Nominal(name, _), ConstValue::Tag { name: cv_name, .. }) if name == cv_name => {
            true
        }
        (Pattern::Qualified(path), ConstValue::Tag { name: cv_name, .. }) => {
            path.segments.last() == Some(cv_name)
        }
        (
            Pattern::Generic {
                name,
                params,
                param_spans,
                ..
            },
            ConstValue::Tag {
                name: cv_name,
                args,
                ..
            },
        ) => {
            name == cv_name
                && param_spans.len() == args.len()
                && param_spans
                    .iter()
                    .zip(args.iter())
                    .all(|((pname, _), arg)| {
                        params
                            .iter()
                            .find(|(n, _)| n == pname)
                            .is_some_and(|(_, kind)| pattern_param_matches(pname, kind, arg))
                    })
        }
        (Pattern::ListCons { head, tail }, ConstValue::Sequence(items)) => {
            if items.is_empty() {
                return false;
            }
            if !pattern_matches(&head.value, &items[0]) {
                return false;
            }
            let rest = ConstValue::Sequence(items[1..].to_vec().into());
            pattern_matches(&tail.value, &rest)
        }
        (Pattern::ListEmpty, ConstValue::Sequence(items)) => items.is_empty(),
        (Pattern::Nominal(tag_name, _), ConstValue::Tag { name: cv_name, .. }) => {
            tag_name == cv_name
        }
        (Pattern::Nominal(_tag_name, _), ConstValue::Record { .. }) => {
            // Match a type's constructor pattern against a record.
            true
        }
        (Pattern::ListEmpty, cv) => {
            // `[]` — empty list pattern
            matches!(cv, ConstValue::Sequence(items) if items.is_empty())
        }
        _ => false,
    }
}

fn pattern_param_matches(name: &Intern<String>, kind: &ParameterKind, cv: &ConstValue) -> bool {
    match kind {
        ParameterKind::Proposition(_) => matches!(cv, ConstValue::Proposition(_)),
        ParameterKind::Generic => {
            name.as_str().starts_with_lowercase()
                || name.as_str() == "_"
                || matches!(cv, ConstValue::Tag { name: cv_name, .. } if name == cv_name)
        }
        ParameterKind::Tagged(sp)
        | ParameterKind::ValueParam { ty: sp }
        | ParameterKind::Inferred { ty: sp } => {
            let pattern = Pattern::from_expr(sp.value.clone());
            pattern_matches(&pattern, cv)
        }
        ParameterKind::Default(expr) => default_expr_matches(&expr.value, cv),
    }
}

fn default_expr_matches(expr: &Expr, cv: &ConstValue) -> bool {
    match (expr, cv) {
        (
            Expr::TagCall(call),
            ConstValue::Tag {
                name,
                qual_path,
                args,
            },
        ) => {
            call.name == *name
                && call
                    .qualifier()
                    .is_none_or(|prefix| qual_path.as_deref() == Some(prefix.as_str()))
                && call.args.len() == args.len()
                && call
                    .args
                    .iter()
                    .zip(args.iter())
                    .all(|(expr, value)| default_expr_matches(&expr.value, value))
        }
        (Expr::AnonymousTag(name), ConstValue::Tag { name: cv_name, .. }) => name == cv_name,
        (Expr::FnCall(call), ConstValue::Tag { name: cv_name, .. })
            if call.args.as_ref().is_none_or(|args| args.is_empty())
                && call.path.value.segments.is_empty() =>
        {
            call.path.value.root == *cv_name
        }
        (Expr::Lit(Literal::Int(n)), ConstValue::Int(cv_n)) => *n == *cv_n,
        (Expr::Lit(Literal::Float(HashFloat(f))), ConstValue::Float(HashFloat(cv_f))) => {
            f.to_bits() == cv_f.to_bits()
        }
        (Expr::Lit(Literal::String(s)), ConstValue::Text(cv_s)) => s == cv_s,
        _ => false,
    }
}

/// Check if a tag name looks like a lowercase type variable.
/// Extract pattern bindings from matching a pattern against a value.
///
/// For example, matching `Some(v)` against `Some(3)` pushes `v → 3` into the env.
pub(crate) fn collect_pattern_bindings(
    pattern: &Pattern,
    cv: &ConstValue,
    env: &mut HashMap<Intern<String>, ConstValue>,
) {
    collect_pattern_bindings_inner(pattern, cv, env);
}

fn collect_pattern_bindings_inner(
    pattern: &Pattern,
    cv: &ConstValue,
    env: &mut HashMap<Intern<String>, ConstValue>,
) {
    match (pattern, cv) {
        (Pattern::Nominal(name, _), _) if name.as_str() != "_" => {
            env.insert(*name, cv.clone());
        }
        (
            Pattern::Generic {
                name,
                params,
                param_spans,
                ..
            },
            ConstValue::Tag {
                name: cv_name,
                args,
                ..
            },
        ) if name == cv_name => {
            for ((pname, _), arg) in param_spans.iter().zip(args.iter()) {
                let kind = params
                    .iter()
                    .find(|(n, _)| n == pname)
                    .map(|(_, k)| k)
                    .expect("a matching constructor pattern has metadata for every argument");
                collect_pattern_param_binding(pname, kind, arg, env);
            }
        }
        (Pattern::ListCons { head, tail }, ConstValue::Sequence(items)) if !items.is_empty() => {
            collect_pattern_bindings_inner(&head.value, &items[0], env);
            let rest = ConstValue::Sequence(items[1..].into());
            collect_pattern_bindings_inner(&tail.value, &rest, env);
        }
        (Pattern::ListEmpty, _) => {}
        (
            Pattern::Nominal(tag_name, _),
            ConstValue::Tag {
                name: cv_name,
                args,
                ..
            },
        ) if tag_name == cv_name => {
            let _ = args;
        }
        _ => {}
    }
}

fn collect_pattern_param_binding(
    name: &Intern<String>,
    kind: &ParameterKind,
    cv: &ConstValue,
    env: &mut HashMap<Intern<String>, ConstValue>,
) {
    match kind {
        ParameterKind::Generic if name.as_str() != "_" && name.as_str().starts_with_lowercase() => {
            env.insert(*name, cv.clone());
        }
        ParameterKind::Tagged(sp)
        | ParameterKind::ValueParam { ty: sp }
        | ParameterKind::Inferred { ty: sp } => {
            let pattern = Pattern::from_expr(sp.value.clone());
            collect_pattern_bindings_inner(&pattern, cv, env)
        }
        ParameterKind::Proposition(_) => {
            if name.as_str() != "_" {
                env.insert(*name, cv.clone());
            }
        }
        ParameterKind::Generic | ParameterKind::Default(_) => {}
    }
}
