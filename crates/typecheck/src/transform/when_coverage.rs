use ast::parameter::ParameterKind;
use ast::{Condition, Expr, FnCall, ModPath, Pattern, SpanId, Spanned, TagCall, Typed};
use internment::Intern;

type PatternPath<'a> = Vec<(&'a Typed<Expr>, &'a Pattern)>;
const COVERAGE_PATHS_MAX: usize = 4096;
const COVERAGE_NODES_MAX: usize = 65_536;

pub(super) fn condition_coverage(condition: &Condition, subject: Option<SpanId>) -> Vec<Pattern> {
    let Some(subject) = subject else {
        return Vec::new();
    };
    let mut pending = vec![(condition, false)];
    let mut results: Vec<Vec<PatternPath<'_>>> = Vec::new();
    for _ in 0..COVERAGE_NODES_MAX {
        let Some((condition, visited)) = pending.pop() else {
            break;
        };
        match condition {
            Condition::Is { subject, pattern } => {
                results.push(vec![vec![(subject, &pattern.value)]])
            }
            Condition::And(left, right) | Condition::Or(left, right) if !visited => {
                pending.push((condition, true));
                pending.push((right, false));
                pending.push((left, false));
            }
            Condition::And(..) => {
                let right = results.pop().unwrap();
                let left = results.pop().unwrap();
                if left.len().saturating_mul(right.len()) > COVERAGE_PATHS_MAX {
                    return Vec::new();
                }
                results.push(
                    left.into_iter()
                        .flat_map(|left| {
                            right.iter().map(move |right| {
                                let mut path = left.clone();
                                path.extend(right);
                                path
                            })
                        })
                        .collect(),
                );
            }
            Condition::Or(..) => {
                let right = results.pop().unwrap();
                let left = results.last_mut().unwrap();
                if left.len() + right.len() > COVERAGE_PATHS_MAX {
                    return Vec::new();
                }
                left.extend(right);
            }
            _ => results.push(Vec::new()),
        }
    }
    if !pending.is_empty() {
        return Vec::new();
    }
    assert_eq!(results.len(), 1);
    results
        .pop()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| compose_path(&path, subject))
        .collect()
}

fn compose_path(path: &PatternPath<'_>, subject: SpanId) -> Option<Pattern> {
    let (root, pattern) = path.first()?;
    if root.span_id != subject {
        return None;
    }
    let mut root = (*pattern).clone();
    for (subject, pattern) in &path[1..] {
        let Expr::FnCall(call) = &subject.value else {
            return None;
        };
        if call.args.is_some() || !call.path.value.segments.is_empty() {
            return None;
        }
        let replacement = pattern_expression(pattern)?;
        if !substitute_binding(&mut root, call.path.value.root, replacement) {
            return None;
        }
    }
    Some(root)
}

fn pattern_expression(pattern: &Pattern) -> Option<Typed<Expr>> {
    let span = SpanId::INVALID;
    let value = match pattern {
        Pattern::Nominal(name, _) => Expr::FnCall(FnCall {
            syntax: ast::CallSyntax::Path,
            definition: None,
            path: Spanned::new(ModPath::new(*name, Vec::new()), span),
            args: None,
        }),
        Pattern::Qualified(path) => Expr::FnCall(FnCall {
            syntax: ast::CallSyntax::Path,
            definition: None,
            path: path.clone(),
            args: None,
        }),
        Pattern::Literal(value, _) => Expr::Lit(value.clone()),
        Pattern::Generic {
            name,
            params,
            param_spans,
            ..
        } => {
            let args = param_spans
                .iter()
                .map(|(parameter, _)| {
                    let (_, kind) = params.iter().find(|(name, _)| name == parameter)?;
                    match kind {
                        ParameterKind::Generic => Some(Typed::infer(
                            Expr::FnCall(FnCall {
                                syntax: ast::CallSyntax::Path,
                                definition: None,
                                path: Spanned::new(ModPath::new(*parameter, Vec::new()), span),
                                args: None,
                            }),
                            span,
                        )),
                        ParameterKind::Default(expr) => Some(*expr.clone()),
                        ParameterKind::Tagged(expr)
                        | ParameterKind::ValueParam { ty: expr }
                        | ParameterKind::Inferred { ty: expr } => {
                            Some(Typed::infer(expr.value.clone(), expr.span_id))
                        }
                        ParameterKind::Proposition(_) => None,
                    }
                })
                .collect::<Option<Vec<_>>>()?;
            Expr::TagCall(TagCall {
                name: *name,
                qual_path: None,
                args,
            })
        }
        _ => return None,
    };
    Some(Typed::infer(value, span))
}

fn substitute_binding(
    pattern: &mut Pattern,
    binding: Intern<String>,
    replacement: Typed<Expr>,
) -> bool {
    if matches!(pattern, Pattern::Nominal(name, _) if *name == binding) {
        *pattern = Pattern::from_expr(replacement.value);
        return true;
    }
    let Pattern::Generic { params, .. } = pattern else {
        return false;
    };
    let mut expressions = Vec::new();
    for (name, kind) in params {
        match kind {
            ParameterKind::Generic if *name == binding => {
                *kind = ParameterKind::Default(Box::new(replacement));
                return true;
            }
            ParameterKind::Default(expr) => expressions.push(&mut expr.value),
            ParameterKind::Tagged(expr)
            | ParameterKind::ValueParam { ty: expr }
            | ParameterKind::Inferred { ty: expr } => expressions.push(&mut expr.value),
            _ => {}
        }
    }
    for _ in 0..COVERAGE_NODES_MAX {
        let Some(expr) = expressions.pop() else {
            break;
        };
        match expr {
            Expr::FnCall(call)
                if call.args.is_none()
                    && call.path.value.segments.is_empty()
                    && call.path.value.root == binding =>
            {
                *expr = replacement.value;
                return true;
            }
            Expr::TagCall(call) => {
                expressions.extend(call.args.iter_mut().map(|arg| &mut arg.value))
            }
            _ => {}
        }
    }
    false
}
