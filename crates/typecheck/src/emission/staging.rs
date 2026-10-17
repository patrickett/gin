use diagnostic::code::EvaluationCode;
use std::collections::HashSet;
use std::ops::ControlFlow;

use ast::ConstValue;
use diagnostic::Diagnostic;
use flask::CompileTarget;
use internment::Intern;

use super::EmissionContext;
use crate::eval::{EvalFailure, EvalOutcome};
use crate::typed::{BindBody, ExprId, TypedExprKind, typed_walk_expr_children};

use super::error;

fn children(program: crate::ResolvedProgram<'_>, id: ExprId) -> Vec<ExprId> {
    let mut children = Vec::new();
    if let Some(kind) = program.exprs.kind_of(id) {
        let _ = typed_walk_expr_children(kind, &mut |id| {
            children.push(id);
            ControlFlow::Continue(())
        });
    }
    children
}

fn prefix_to(
    program: crate::ResolvedProgram<'_>,
    root: ExprId,
    target: ExprId,
    prefix: &mut Vec<ExprId>,
) -> bool {
    if root == target {
        return true;
    }
    let before = prefix.len();
    if let Some(TypedExprKind::Bind {
        stmts,
        body,
        unassigned,
        ..
    }) = program.exprs.kind_of(root)
    {
        for statement in stmts {
            if prefix_to(program, *statement, target, prefix) {
                return true;
            }
            prefix.push(*statement);
        }
        if !unassigned && prefix_to(program, *body, target, prefix) {
            return true;
        }
    } else {
        for child in children(program, root) {
            if prefix_to(program, child, target, prefix) {
                return true;
            }
        }
    }
    prefix.truncate(before);
    false
}

fn names(
    program: crate::ResolvedProgram<'_>,
    roots: &[ExprId],
    skip: &HashSet<ExprId>,
    runtime: bool,
) -> HashSet<Intern<String>> {
    let mut names = HashSet::new();
    let mut visited = HashSet::new();
    let mut pending = roots.to_vec();
    while let Some(id) = pending.pop() {
        if skip.contains(&id) || !visited.insert(id) {
            continue;
        }
        match program.exprs.kind_of(id) {
            Some(
                TypedExprKind::FnCall {
                    target, args: None, ..
                }
                | TypedExprKind::SelfRef { target },
            ) => {
                names.insert(target.0);
            }
            Some(TypedExprKind::Bind { name, .. } | TypedExprKind::Reassign { name, .. }) => {
                names.insert(*name);
            }
            Some(TypedExprKind::Emit { record, .. }) => {
                if runtime {
                    pending.push(*record);
                }
                continue;
            }
            _ => {}
        }
        pending.extend(children(program, id));
    }
    names
}

pub(crate) fn evaluate(
    context: EmissionContext<'_>,
    emission: ExprId,
    specification: ExprId,
    target: &CompileTarget,
) -> Result<(ConstValue, Vec<ExprId>), Box<EvalFailure>> {
    let program = context.program;
    let mut prefix = Vec::new();
    let mut roots = Vec::new();
    for bind in program.defs.values() {
        let body: Vec<_> = match &bind.body {
            BindBody::Expr(expr) => vec![*expr],
            BindBody::Body { exprs, ret } => exprs.iter().copied().chain(*ret).collect(),
            BindBody::Unlowered => continue,
        };
        prefix.clear();
        let mut found = false;
        for root in &body {
            if prefix_to(program, *root, emission, &mut prefix) {
                found = true;
                break;
            }
            prefix.push(*root);
        }
        if found {
            roots = body;
            break;
        }
    }
    if roots.is_empty() {
        prefix.clear();
    }
    let empty = HashSet::new();
    let mut required = names(program, &[specification], &empty, false);
    let dependencies: Vec<_> = prefix
        .iter()
        .map(|statement| names(program, &[*statement], &empty, false))
        .collect();
    loop {
        let before = required.len();
        for dependencies in &dependencies {
            if !required.is_disjoint(dependencies) {
                required.extend(dependencies.iter().copied());
            }
        }
        if before == required.len() {
            break;
        }
    }
    let construction: Vec<_> = prefix
        .into_iter()
        .zip(dependencies)
        .filter_map(|(statement, dependencies)| {
            (!required.is_disjoint(&dependencies)).then_some(statement)
        })
        .collect();
    let skip: HashSet<_> = construction.iter().copied().collect();
    let locals: HashSet<_> = construction
        .iter()
        .filter_map(|id| match program.exprs.kind_of(*id) {
            Some(TypedExprKind::Bind { name, .. }) => Some(*name),
            _ => None,
        })
        .collect();
    if !locals.is_disjoint(&names(program, &roots, &skip, true)) {
        return Err(Box::new(
            error("code construction cannot share mutable local state with runtime expressions")
                .into(),
        ));
    }
    let mut evaluator =
        crate::eval::Evaluator::new(program, crate::eval::CompileTimeHost::new(target))
            .with_fuel(crate::eval::Fuel::new(100_000))
            .with_programs(context.programs());
    let outcome = evaluator.eval_after(&construction, specification);
    let diagnostic = match outcome {
        Ok(EvalOutcome::Complete(value)) => return Ok((value, construction)),
        Ok(EvalOutcome::Trap(diagnostic)) | Err(diagnostic) => diagnostic,
        // compile-time-cancelled
        Ok(EvalOutcome::Cancelled) => Diagnostic::new(
            EvaluationCode::Cancelled,
            "compile-time evaluation was cancelled",
        ),
        Ok(
            EvalOutcome::RuntimeDependency(_) | EvalOutcome::Residual(_) | EvalOutcome::Address(_),
        ) => error("#emit code construction depends on a runtime value"),
    };
    Err(Box::new(evaluator.take_failure(diagnostic)))
}

#[cfg(test)]
mod tests {
    use super::prefix_to;
    use crate::{FileId, TypedExprKind};

    #[test]
    fn unassigned_binding_has_no_body_to_search() {
        let parsed = parser::parse_from_str(
            "Byte is in 0 to 255\nByte has finish(self): return\nmain:\n    1\n    value Byte\n    return 2\n",
        );
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let typed = crate::transform::transform_file(parsed, FileId(0));
        let (binding, placeholder) = typed
            .exprs
            .kind
            .iter()
            .enumerate()
            .find_map(|(index, kind)| match kind {
                TypedExprKind::Bind {
                    body,
                    unassigned: true,
                    ..
                } => Some((crate::ExprId(index as u32), *body)),
                _ => None,
            })
            .expect("unassigned binding");
        assert_ne!(binding, placeholder);
        let mut prefix = Vec::new();
        assert!(!prefix_to(
            typed.resolved_program(),
            binding,
            placeholder,
            &mut prefix
        ));
        assert!(prefix.is_empty());
    }
}
