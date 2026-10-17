use ast::parameter::ParameterKind;
use ast::{Condition, Expr, ModPath, Pattern, SpanId, Spanned, Typed};
use internment::Intern;

use super::path_call;

const PATTERN_NODES_MAX: usize = 4096;

pub(super) fn pattern_condition(
    subject: Typed<Expr>,
    pattern: Spanned<Pattern>,
) -> Option<Condition> {
    let mut pending = vec![(subject, pattern)];
    let mut conditions = Vec::new();
    for _ in 0..PATTERN_NODES_MAX {
        let Some((subject, mut pattern)) = pending.pop() else {
            break;
        };
        let mut nested = Vec::new();
        if let Pattern::Generic {
            params,
            param_spans,
            ..
        } = &mut pattern.value
        {
            assert_eq!(params.len(), param_spans.len());
            for (slot, (name, kind)) in params.iter_mut().enumerate() {
                let ParameterKind::Default(expr) = kind else {
                    continue;
                };
                if !matches!(expr.value, Expr::TagCall(_)) {
                    continue;
                }
                let span = expr.span_id;
                let mut nested_pattern = Pattern::from_expr(expr.value.clone());
                if let Pattern::Generic {
                    span: name_span, ..
                } = &mut nested_pattern
                {
                    *name_span = span;
                }
                let binding = Intern::new(format!("when$payload_{}_{}", span.into_inner(), slot));
                *name = binding;
                param_spans[slot] = (binding, SpanId::INVALID);
                *kind = ParameterKind::Generic;
                let subject = Typed::infer(
                    path_call(Spanned::new(ModPath::new(binding, vec![]), SpanId::INVALID)),
                    SpanId::INVALID,
                );
                nested.push((subject, Spanned::new(nested_pattern, span)));
            }
        }
        conditions.push(Condition::Is {
            subject: Box::new(subject),
            pattern: Box::new(pattern),
        });
        pending.extend(nested.into_iter().rev());
    }
    if !pending.is_empty() {
        return None;
    }
    assert!(!conditions.is_empty());
    let mut conditions = conditions.into_iter();
    let first = conditions.next().expect("a pattern has a root condition");
    Some(conditions.fold(first, |left, right| {
        Condition::And(Box::new(left), Box::new(right))
    }))
}
