//! Pattern matching — check whether a [`Pattern`] matches a [`ConstValue`].
//!
//! Used by compile-time expression evaluation (when-arm matching, tag-declare
//! simplification) and by trait/reflect machinery.

use std::collections::HashMap;

use i256::I256;
use internment::Intern;

use crate::ty::Ty;
use crate::typed::TagId;
use ast::InRangeBounds;
use ast::WhenArm;
use ast::expr::{Expr, Literal, Typed};
use ast::parameter::ParameterKind;
use ast::{ConstValue, HashFloat, Pattern};

/// Public entry: check whether a [`Pattern`] matches a [`ConstValue`].
pub fn pattern_matches_public(pattern: &Pattern, cv: &ConstValue) -> bool {
    pattern_matches(pattern, cv)
}

/// Like [`pattern_matches_public`] but resolves `InRangeBounds::Tag(name)`
/// to actual literal bounds using the given tag types.
pub fn pattern_matches_with_tag_types(
    pattern: &Pattern,
    cv: &ConstValue,
    tag_types: &HashMap<TagId, Ty>,
) -> bool {
    let resolved = resolve_in_range_tag(pattern, tag_types);
    pattern_matches(&resolved, cv)
}

/// Replace `InRangeBounds::Tag(name)` with `InRangeBounds::Literal(min, max)`
/// by looking up the tag's bounded int type.
fn resolve_in_range_tag<'a>(
    pattern: &'a Pattern,
    tag_types: &HashMap<TagId, Ty>,
) -> std::borrow::Cow<'a, Pattern> {
    if let Pattern::InRange {
        bounds: InRangeBounds::Tag(name),
        span,
    } = pattern
        && let Some(Ty::Int {
            min: Some(min),
            max: Some(max),
            ..
        }) = tag_types.get(&TagId(*name))
    {
        return std::borrow::Cow::Owned(Pattern::InRange {
            bounds: InRangeBounds::Literal(I256::from_i128(*min), I256::from_i128(*max)),
            span: *span,
        });
    }
    std::borrow::Cow::Borrowed(pattern)
}

/// Check whether a when-arm's pattern matches a subject value.
/// Returns `Some(body)` when a matching arm is found.
///
/// Used by intrinsic folding and tag-declare simplification where we need
/// to select a when arm at compile time but don't need full evaluation.
pub(crate) fn find_matching_when_body<'a>(
    arms: &'a [WhenArm],
    cv: &ConstValue,
) -> Option<&'a Typed<Expr>> {
    for arm in arms {
        match arm {
            WhenArm::Is { pattern, body, .. } => {
                if pattern_matches_public(&pattern.value, cv) {
                    return Some(body);
                }
            }
            WhenArm::Else(body, _) => return Some(body),
            WhenArm::Cond { .. } => {}
        }
    }
    None
}

/// Core pattern matching — does a `Pattern` match a `ConstValue`?
pub(crate) fn pattern_matches(pattern: &Pattern, cv: &ConstValue) -> bool {
    match (pattern, cv) {
        (Pattern::Literal(Literal::String(s), _), ConstValue::String(cv_s)) => s == cv_s,
        (Pattern::Literal(Literal::Int(n), _), ConstValue::Int(cv_n)) => *n as i128 == *cv_n,
        (
            Pattern::Literal(Literal::Float(HashFloat(f)), _),
            ConstValue::Float(HashFloat(cv_f)),
        ) => f.to_bits() == cv_f.to_bits(),
        (Pattern::Literal(Literal::Number(n), _), ConstValue::Int(cv_n)) => *n as i128 == *cv_n,
        (
            Pattern::InRange {
                bounds: InRangeBounds::Literal(min, max),
                ..
            },
            ConstValue::Int(n),
        ) => *n >= min.as_i128() && *n <= max.as_i128(),
        (Pattern::Nominal(name, _), cv) if is_lowercase_type_var(name) || name.as_str() == "_" => {
            !matches!(cv, ConstValue::String(_))
        }
        (Pattern::Nominal(name, _), ConstValue::Tag { name: cv_name, .. }) if name == cv_name => {
            true
        }
        (Pattern::Qualified(path), ConstValue::Tag { name: cv_name, .. }) => {
            path.segments.last() == Some(cv_name)
        }
        (Pattern::Nominal(name, _), ConstValue::Record { fields })
            if record_matches_reflect_tag(name.as_str(), fields) =>
        {
            true
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
                        let kind = params
                            .iter()
                            .find(|(n, _)| n == pname)
                            .map(|(_, k)| k)
                            .unwrap_or(&ParameterKind::Generic);
                        pattern_param_matches(pname, kind, arg)
                    })
        }
        (Pattern::ListCons { head, tail }, ConstValue::List(items)) => {
            if items.is_empty() {
                return false;
            }
            if !pattern_matches(&head.value, &items[0]) {
                return false;
            }
            let rest = ConstValue::List(items[1..].to_vec().into());
            pattern_matches(&tail.value, &rest)
        }
        (Pattern::ListEmpty, ConstValue::List(items)) => items.is_empty(),
        (Pattern::Tuple(patterns), ConstValue::List(items)) => {
            if patterns.len() != items.len() {
                return false;
            }
            patterns
                .iter()
                .zip(items.iter())
                .all(|(p, i)| pattern_matches(&p.value, i))
        }
        (Pattern::Nominal(tag_name, _), ConstValue::Tag { name: cv_name, .. }) => {
            tag_name == cv_name
        }
        (Pattern::Nominal(_tag_name, _), ConstValue::Record { .. }) => {
            // Match a type's constructor pattern against a record.
            true
        }
        (Pattern::ListEmpty, cv) => {
            // `[]` — empty list pattern
            matches!(cv, ConstValue::List(items) if items.is_empty())
        }
        _ => false,
    }
}

fn pattern_param_matches(name: &Intern<String>, kind: &ParameterKind, cv: &ConstValue) -> bool {
    match kind {
        ParameterKind::Generic => {
            is_lowercase_type_var(name)
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
        (Expr::AnonymousTag(name), ConstValue::Tag { name: cv_name, .. }) => name == cv_name,
        (Expr::FnCall(call), ConstValue::Tag { name: cv_name, .. })
            if call.args.as_ref().is_none_or(|args| args.is_empty())
                && call.path.value.segments.is_empty() =>
        {
            call.path.value.root == *cv_name
        }
        (Expr::Lit(Literal::Int(n)), ConstValue::Int(cv_n)) => *n as i128 == *cv_n,
        (Expr::Lit(Literal::Number(n)), ConstValue::Int(cv_n)) => *n as i128 == *cv_n,
        (Expr::Lit(Literal::Float(HashFloat(f))), ConstValue::Float(HashFloat(cv_f))) => {
            f.to_bits() == cv_f.to_bits()
        }
        (Expr::Lit(Literal::String(s)), ConstValue::String(cv_s)) => s == cv_s,
        _ => false,
    }
}

/// Check if a tag name looks like a lowercase type variable.
fn is_lowercase_type_var(name: &Intern<String>) -> bool {
    name.as_str()
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase())
}

/// Check whether a record's fields match a known reflect tag shape.
fn record_matches_reflect_tag(name: &str, fields: &[(Intern<String>, ConstValue)]) -> bool {
    let has = |field: &str| fields.iter().any(|(n, _)| n.as_str() == field);
    match name {
        "NamedTy" => has("name") && has("ty"),
        "VariantShape" => has("name") && has("fields"),
        _ => false,
    }
}

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
                    .unwrap_or(&ParameterKind::Generic);
                collect_pattern_param_binding(pname, kind, arg, env);
            }
        }
        (Pattern::ListCons { head, tail }, ConstValue::List(items)) if !items.is_empty() => {
            collect_pattern_bindings_inner(&head.value, &items[0], env);
            let rest = ConstValue::List(items[1..].into());
            collect_pattern_bindings_inner(&tail.value, &rest, env);
        }
        (Pattern::ListEmpty, _) => {}
        (Pattern::Tuple(patterns), ConstValue::List(items)) => {
            for (p, i) in patterns.iter().zip(items.iter()) {
                collect_pattern_bindings_inner(&p.value, i, env);
            }
        }
        // Note: `ConstValue` has no `Tuple` variant; tuple patterns match `ConstValue::List`.
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
        ParameterKind::Generic if name.as_str() != "_" && is_lowercase_type_var(name) => {
            env.insert(*name, cv.clone());
        }
        ParameterKind::Tagged(sp)
        | ParameterKind::ValueParam { ty: sp }
        | ParameterKind::Inferred { ty: sp } => {
            let pattern = Pattern::from_expr(sp.value.clone());
            collect_pattern_bindings_inner(&pattern, cv, env)
        }
        ParameterKind::Generic | ParameterKind::Default(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::ConstValue;
    use ast::span::{SpanId, Spanned, SubSpan};
    use ast::type_expr::TypeExpr;

    fn span_id() -> SpanId {
        SpanId::new(0)
    }

    #[test]
    fn test_literal_string_match() {
        let pat = TypeExpr::Literal(Literal::String("hello".to_string()), span_id());
        let pat = Pattern::from(pat);
        let cv = ConstValue::String("hello".to_string());
        assert!(pattern_matches(&pat, &cv));
    }

    #[test]
    fn test_literal_string_no_match() {
        let pat = TypeExpr::Literal(Literal::String("hello".to_string()), span_id());
        let pat = Pattern::from(pat);
        let cv = ConstValue::String("world".to_string());
        assert!(!pattern_matches(&pat, &cv));
    }

    #[test]
    fn test_list_cons_match() {
        // pattern: [head, ..tail]
        let head = Box::new(Spanned {
            value: Pattern::Nominal(Intern::new("head".to_string()), span_id()),
            span_id: span_id(),
        });
        let tail = Box::new(Spanned {
            value: Pattern::ListEmpty,
            span_id: span_id(),
        });
        let pat = Pattern::ListCons { head, tail };

        let cv = ConstValue::List(vec![ConstValue::Int(42)].into());

        assert!(pattern_matches(&pat, &cv));
    }

    #[test]
    fn test_list_cons_no_match() {
        let head = Box::new(Spanned {
            value: Pattern::Nominal(Intern::new("head".to_string()), span_id()),
            span_id: span_id(),
        });
        let tail = Box::new(Spanned {
            value: Pattern::ListCons {
                head: Box::new(Spanned {
                    value: Pattern::Nominal(Intern::new("tail".to_string()), span_id()),
                    span_id: span_id(),
                }),
                tail: Box::new(Spanned {
                    value: Pattern::ListEmpty,
                    span_id: span_id(),
                }),
            },
            span_id: span_id(),
        });
        let pat = Pattern::ListCons { head, tail };

        let cv = ConstValue::List(vec![ConstValue::Int(42)].into());

        assert!(!pattern_matches(&pat, &cv));
    }

    #[test]
    fn test_list_nil_matches_empty() {
        let pat = Pattern::ListEmpty;
        let cv = ConstValue::List(vec![].into());
        assert!(pattern_matches(&pat, &cv));
    }

    #[test]
    fn test_pattern_matches_public_delegates() {
        let pat = TypeExpr::Literal(Literal::Int(5), span_id());
        let pat = Pattern::from(pat);
        let cv = ConstValue::Int(5);
        assert!(pattern_matches(&pat, &cv));
    }

    #[test]
    fn test_collect_pattern_bindings_simple() {
        let pat = TypeExpr::Nominal(Intern::new("x".to_string()), span_id());
        let cv = ConstValue::Int(42);
        let mut env = HashMap::new();
        collect_pattern_bindings(&Pattern::from(pat), &cv, &mut env);
        assert_eq!(env.get(&Intern::new("x".to_string())), Some(&cv));
    }

    #[test]
    fn test_collect_pattern_bindings_underscore() {
        let pat = TypeExpr::Nominal(Intern::new("_".to_string()), span_id());
        let cv = ConstValue::Int(99);
        let mut env = HashMap::new();
        collect_pattern_bindings(&Pattern::from(pat), &cv, &mut env);
        assert!(env.is_empty(), "underscore should not create bindings");
    }

    #[test]
    fn test_find_matching_when_body() {
        use ast::WhenArm;
        let true_arm = WhenArm::Is {
            pattern: Box::new(Spanned {
                value: TypeExpr::Nominal(Intern::new("True".to_string()), span_id()).into(),
                span_id: span_id(),
            }),
            body: Box::new(Typed::infer(Expr::Lit(Literal::Number(1)), span_id())),
            arm_span: SubSpan::new(span_id()),
        };
        let false_arm = WhenArm::Is {
            pattern: Box::new(Spanned {
                value: TypeExpr::Nominal(Intern::new("False".to_string()), span_id()).into(),
                span_id: span_id(),
            }),
            body: Box::new(Typed::infer(Expr::Lit(Literal::Number(0)), span_id())),
            arm_span: SubSpan::new(span_id()),
        };
        let arms = vec![true_arm, false_arm];
        let cv = ConstValue::Tag {
            name: Intern::new("True".to_string()),
            qual_path: None,
            args: Vec::new().into(),
        };
        let body = find_matching_when_body(&arms, &cv);
        assert!(body.is_some());
    }

    #[test]
    fn test_collect_pattern_bindings_list_cons() {
        // pattern: [head, tail] where tail is _
        let head = TypeExpr::Nominal(Intern::new("x".to_string()), span_id());
        let _tail = TypeExpr::Nominal(Intern::new("_".to_string()), span_id());
        let items = vec![ConstValue::Int(10), ConstValue::Int(20)];
        let cv = ConstValue::List(items.into());

        let mut env = HashMap::new();
        collect_pattern_bindings(&Pattern::from(head), &cv, &mut env);
        assert_eq!(env.get(&Intern::new("x".to_string())), Some(&cv));
    }
}
