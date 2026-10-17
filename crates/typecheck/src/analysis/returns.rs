use crate::typed::{BindBody, ExprId, TypedExprKind, TypedFileAst, TypedWhenArm};

pub(crate) fn return_expressions(typed: &TypedFileAst, body: &BindBody) -> Vec<ExprId> {
    let roots = match body {
        BindBody::Expr(expr) => vec![(*expr, true)],
        BindBody::Body { exprs, ret } => exprs
            .iter()
            .map(|id| (*id, false))
            .chain(ret.map(|id| (id, true)))
            .collect(),
        BindBody::Unlowered => return Vec::new(),
    };
    let mut returns = Vec::new();
    for root in roots {
        let mut pending = vec![root];
        let mut visited = std::collections::HashSet::new();
        while let Some((expr, value)) = pending.pop() {
            if !value && !visited.insert(expr) {
                continue;
            }
            if typed.local_callable_parameters.contains_key(&expr) {
                continue;
            }
            match typed.exprs.kind_of(expr) {
                Some(TypedExprKind::Return { value, .. }) => {
                    if let Some(value) = value {
                        pending.push((*value, true));
                    }
                }
                Some(TypedExprKind::Block {
                    statements,
                    value: result,
                }) => {
                    pending.extend(result.map(|id| (id, value)));
                    pending.extend(statements.iter().rev().map(|id| (*id, false)));
                }
                Some(TypedExprKind::When(branch)) => {
                    for arm in branch.arms.iter().rev() {
                        let body = match arm {
                            TypedWhenArm::Cond { body, .. }
                            | TypedWhenArm::Is { body, .. }
                            | TypedWhenArm::Else(body, _) => *body,
                        };
                        pending.push((body, value));
                    }
                }
                Some(TypedExprKind::Bind { stmts, body, .. }) => {
                    pending.push((*body, value));
                    pending.extend(stmts.iter().rev().map(|id| (*id, false)));
                }
                Some(TypedExprKind::If(branch)) => {
                    pending.extend(branch.ret.map(|id| (id, true)));
                    pending.extend(branch.stmts.iter().rev().map(|id| (*id, false)));
                }
                Some(kind) => {
                    if value && !expression_returns(typed.resolved_program(), expr) {
                        returns.push(expr);
                    }
                    let _ = crate::typed::typed_walk_expr_children(kind, &mut |child| {
                        pending.push((child, false));
                        std::ops::ControlFlow::Continue(())
                    });
                }
                None if value => returns.push(expr),
                None => {}
            }
        }
    }
    returns
}

pub fn expression_returns(typed: crate::ResolvedProgram<'_>, root: ExprId) -> bool {
    let mut results = std::collections::HashMap::new();
    let mut pending = vec![(root, false)];
    while let Some((id, visited)) = pending.pop() {
        if results.contains_key(&id) {
            continue;
        }
        if typed.local_callable_parameters.contains_key(&id) {
            results.insert(id, false);
            continue;
        }
        let kind = &typed.exprs.kind[id.index()];
        if !visited {
            pending.push((id, true));
            let _ = crate::typed::typed_walk_expr_children(kind, &mut |child| {
                pending.push((child, false));
                std::ops::ControlFlow::Continue(())
            });
            continue;
        }
        let returns = |child: &ExprId| results.get(child).copied().unwrap_or(false);
        let result = kind_returns(typed, kind, returns);
        results.insert(id, result);
    }
    results[&root]
}

pub(crate) fn kind_returns(
    typed: crate::ResolvedProgram<'_>,
    kind: &TypedExprKind,
    returns: impl Fn(&ExprId) -> bool + Copy,
) -> bool {
    match kind {
        TypedExprKind::Return { .. } => true,
        TypedExprKind::Block { statements, value } => {
            statements.iter().any(returns) || value.as_ref().is_some_and(returns)
        }
        TypedExprKind::Bind { stmts, body, .. } => stmts.iter().any(returns) || returns(body),
        TypedExprKind::When(branch) => {
            let subject_ty = branch
                .subject
                .and_then(|id| typed.exprs.ty_of(id))
                .map(|ty| typed.type_registry.resolved_definition_for_type(ty));
            let covered = branch
                .arms
                .iter()
                .any(|arm| matches!(arm, TypedWhenArm::Else(..)))
                || crate::analysis::when_is_exhaustive(
                    subject_ty.as_ref(),
                    &branch.arms,
                    typed.type_registry,
                );
            branch.subject.as_ref().is_some_and(returns)
                || covered
                    && branch.arms.iter().all(|arm| {
                        returns(&match arm {
                            TypedWhenArm::Cond { body, .. }
                            | TypedWhenArm::Is { body, .. }
                            | TypedWhenArm::Else(body, _) => *body,
                        })
                    })
        }
        TypedExprKind::Loop(_) | TypedExprKind::If(_) => false,
        _ => {
            let mut result = false;
            let _ = crate::typed::typed_walk_expr_children(kind, &mut |child| {
                result |= returns(&child);
                std::ops::ControlFlow::Continue(())
            });
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileId;
    use internment::Intern;
    use parser::query::SourceParseExt;

    fn fixture(source: &str) -> TypedFileAst {
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let typed = crate::transform::transform_file(parsed.ast, FileId(0));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        typed
    }

    fn in_child(test: &str) -> bool {
        const CHILD: &str = "GIN_RETURN_COLLECTOR_SMALL_STACK";
        if std::env::var_os(CHILD).is_some() {
            return true;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        false
    }

    #[test]
    fn deeply_nested_value_returns_fit_a_small_stack() {
        if !in_child("analysis::returns::tests::deeply_nested_value_returns_fit_a_small_stack") {
            return;
        }
        let mut typed = fixture("choose(flag is in 0 to 1): return when flag = 1 then 1 else 0\n");
        let index = typed
            .exprs
            .kind
            .iter()
            .position(|kind| matches!(kind, TypedExprKind::When(_)))
            .unwrap();
        let template = typed.exprs.clone().remove(index);
        let TypedExprKind::When(branch) = &template.kind else {
            unreachable!()
        };
        let (TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. }) = &branch.arms[0]
        else {
            panic!("conditional arm")
        };
        let first = *body;
        let TypedWhenArm::Else(last, _) = branch.arms[1] else {
            panic!("else arm")
        };
        let count = 2000;
        let mut root = first;
        for _ in 0..count {
            let mut node = template.clone();
            let TypedExprKind::When(branch) = &mut node.kind else {
                unreachable!()
            };
            let (TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. }) =
                &mut branch.arms[0]
            else {
                unreachable!()
            };
            *body = root;
            root = ExprId(u32::try_from(typed.exprs.kind.len()).unwrap());
            typed.exprs.push(node);
        }
        let returns = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || return_expressions(&typed, &BindBody::Expr(root)))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(returns.len(), count + 1);
        assert_eq!(returns[0], first);
        assert!(returns[1..].iter().all(|value| *value == last));
    }

    #[test]
    fn long_returning_sequences_preserve_each_branch_on_a_small_stack() {
        if !in_child(
            "analysis::returns::tests::long_returning_sequences_preserve_each_branch_on_a_small_stack",
        ) {
            return;
        }
        let typed = fixture(
            "choose(flag is in 0 to 1):\n    when flag = 1 then\n        return 1\n    return 0\n",
        );
        let BindBody::Body {
            exprs,
            ret: Some(last),
        } = &typed.defs[&crate::DefId::named(Intern::from_ref("choose"))].body
        else {
            panic!("block body")
        };
        let first = return_expressions(
            &typed,
            &BindBody::Body {
                exprs: vec![exprs[0]],
                ret: None,
            },
        )[0];
        let last = *last;
        let count = 2000;
        let body = BindBody::Body {
            exprs: vec![exprs[0]; count],
            ret: Some(last),
        };
        let returns = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || return_expressions(&typed, &body))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(returns.len(), count + 1);
        assert!(returns[..count].iter().all(|value| *value == first));
        assert_eq!(returns[count], last);
    }
}
