use std::collections::HashSet;

use crate::typed::{BindBody, ExprId, TypedBind, TypedExprKind, TypedFileAst, TypedPostcondition};
use ast::{PredicateExpr, ProofProposition};

mod result;
mod value;
pub(super) use value::stable as stable_value_input;

pub(super) fn alternative(
    bind: &ast::Bind,
    label: internment::Intern<String>,
) -> Option<ProofProposition> {
    fn exit(
        ret: &ast::Return,
        label: internment::Intern<String>,
        contracts: &mut Vec<Option<ProofProposition>>,
    ) {
        if matches!(ret.values.as_slice(), [value] if match &value.value {
            ast::Expr::AnonymousTag(name) => *name == label,
            ast::Expr::TagCall(call) => call.name == label && call.args.is_empty() && call.qual_path.is_none(),
            _ => false,
        }) {
            contracts.push(ret.postcondition.as_ref().map(|proof| proof.value.clone()));
        }
    }
    fn visit(
        expression: &ast::Expr,
        label: internment::Intern<String>,
        contracts: &mut Vec<Option<ProofProposition>>,
    ) -> bool {
        match expression {
            ast::Expr::Return(ret) => exit(ret, label, contracts),
            ast::Expr::Block(_) | ast::Expr::When(_) => {}
            ast::Expr::Bind(bind) if bind.params.is_some() => return true,
            ast::Expr::If(branch) => {
                exit(&branch.ret, label, contracts);
            }
            ast::Expr::FnCall(call)
                if call.args.is_none() && call.path.value.segments.is_empty() => {}
            ast::Expr::TagCall(call) if call.args.is_empty() && call.qual_path.is_none() => {}
            ast::Expr::Binary(_) | ast::Expr::Lit(_) | ast::Expr::AnonymousTag(_) => {}
            _ => return false,
        }
        ast::folder::walk_expr_children(expression, &mut |_, child| {
            if visit(child, label, contracts) {
                std::ops::ControlFlow::Continue(())
            } else {
                std::ops::ControlFlow::Break(())
            }
        })
        .is_continue()
    }
    let mut contracts = Vec::new();
    match &bind.value {
        ast::BindValue::Body { exprs, ret } => {
            exit(ret, label, &mut contracts);
            if !exprs
                .iter()
                .chain(&ret.values)
                .all(|expression| visit(&expression.value, label, &mut contracts))
            {
                return None;
            }
        }
        ast::BindValue::Expr(expression) => {
            if !visit(&expression.value, label, &mut contracts) {
                return None;
            }
        }
        ast::BindValue::Unassigned => return None,
    }
    let contract = contracts.first()?.as_ref()?;
    if !contracts
        .iter()
        .all(|candidate| candidate.as_ref() == Some(contract))
    {
        return None;
    }
    let predicate = PredicateExpr::Proposition(Box::new(contract.clone()));
    if !super::refinement_terms::predicate_names(&predicate)
        .iter()
        .all(|name| {
            bind.params
                .as_ref()
                .is_some_and(|params| params.contains_key(name))
        })
    {
        return None;
    }
    Some(contract.clone())
}

pub(super) fn common_proof(typed: &TypedFileAst, bind: &TypedBind) -> Option<TypedPostcondition> {
    common_exits(typed, bind).map(|exits| exits.proof)
}

struct CommonExits {
    proof: TypedPostcondition,
    exits: Vec<Option<TypedPostcondition>>,
    seen: HashSet<ExprId>,
}

fn common_exits(typed: &TypedFileAst, bind: &TypedBind) -> Option<CommonExits> {
    fn visit(
        typed: &TypedFileAst,
        bind: &TypedBind,
        expression: ExprId,
        exits: &mut Vec<Option<TypedPostcondition>>,
        seen: &mut HashSet<ExprId>,
    ) {
        if !seen.insert(expression) {
            return;
        }
        let Some(kind) = typed.exprs.kind_of(expression) else {
            return;
        };
        if typed.local_callable_parameters.contains_key(&expression) {
            return;
        }
        if let TypedExprKind::Return {
            value,
            postcondition,
        } = kind
        {
            exits.push(
                postcondition
                    .as_ref()
                    .map(|proof| result::normalize(typed, bind, proof, *value)),
            );
        }
        if let TypedExprKind::If(branch) = kind {
            exits.push(
                branch
                    .postcondition
                    .as_ref()
                    .map(|proof| result::normalize(typed, bind, proof, branch.ret)),
            );
        }
        let _ = crate::typed::walk_expr_children(kind, &mut |child| {
            visit(typed, bind, child, exits, seen);
            std::ops::ControlFlow::Continue(())
        });
    }
    let mut exits = Vec::new();
    let roots: Vec<_> = match &bind.body {
        BindBody::Body { exprs, ret } => {
            if !exprs.iter().any(|expression| {
                crate::analysis::expression_returns(typed.resolved_program(), *expression)
            }) {
                exits.push(
                    bind.postcondition
                        .as_ref()
                        .map(|proof| result::normalize(typed, bind, proof, *ret)),
                );
            }
            exprs.iter().copied().chain(*ret).collect()
        }
        BindBody::Expr(expression) => vec![*expression],
        BindBody::Unlowered => return None,
    };
    let mut seen = HashSet::new();
    for expression in roots {
        visit(typed, bind, expression, &mut exits, &mut seen);
    }
    let Some(proof) = exits.first().and_then(Option::as_ref).cloned() else {
        return None;
    };
    if !exits.iter().all(|exit| {
        exit.as_ref()
            .is_some_and(|exit| exit.proposition == proof.proposition)
    }) {
        return None;
    }
    Some(CommonExits { proof, exits, seen })
}

pub(super) fn common(
    typed: &TypedFileAst,
    bind: &TypedBind,
    signatures: &super::callable_signatures::CallableSignatureScope<'_>,
) -> Vec<ProofProposition> {
    let Some(CommonExits { proof, exits, seen }) = common_exits(typed, bind) else {
        return Vec::new();
    };
    let predicate = PredicateExpr::Proposition(Box::new(proof.proposition.clone()));
    let dependencies = super::refinement_terms::predicate_names(&predicate);
    let has_value_input = dependencies.iter().any(|name| {
        let root = name.as_str().split('.').next().unwrap_or(name.as_str());
        bind.params
            .iter()
            .zip(&bind.param_conventions)
            .any(|((parameter, _), convention)| {
                parameter.as_str() == root
                    && !matches!(
                        convention,
                        ast::ParamConvention::Observe | ast::ParamConvention::Mutate
                    )
            })
    });
    let has_record_value_input = dependencies.iter().any(|name| {
        let root = name.as_str().split('.').next().unwrap_or(name.as_str());
        bind.params
            .iter()
            .zip(&bind.param_conventions)
            .any(|((parameter, ty), convention)| {
                parameter.as_str() == root
                    && !matches!(
                        convention,
                        ast::ParamConvention::Observe | ast::ParamConvention::Mutate
                    )
                    && typed.type_registry.integer_validity_for_type(ty).is_none()
            })
    });
    let stable_inputs = dependencies.iter().all(|name| {
        let root =
            internment::Intern::from_ref(name.as_str().split('.').next().unwrap_or(name.as_str()));
        !bind
            .params
            .iter()
            .zip(&bind.param_conventions)
            .any(|((parameter, _), convention)| {
                *parameter == root
                    && !matches!(
                        convention,
                        ast::ParamConvention::Observe | ast::ParamConvention::Mutate
                    )
            })
            || stable_value_input(typed, bind, root, signatures)
    });
    for expression in &seen {
        match typed.exprs.kind_of(*expression) {
            Some(TypedExprKind::Bind { name, .. } | TypedExprKind::Reassign { name, .. })
                if bind.params.iter().any(|(parameter, _)| parameter == name) =>
            {
                return Vec::new();
            }
            Some(
                TypedExprKind::Reassign { .. }
                | TypedExprKind::AggregateSet { .. }
                | TypedExprKind::RecordSet { .. }
                | TypedExprKind::BufSet { .. }
                | TypedExprKind::Emit { .. },
            ) if has_record_value_input => return Vec::new(),
            Some(TypedExprKind::FnCall { target, args, .. })
                if has_value_input
                    && (args.is_some()
                        || typed.exprs.place_of(*expression).flatten().is_none())
                    && signatures.get(target).is_none_or(|signature| {
                        (signature.effects.unknown_memory && !stable_inputs)
                            || (has_record_value_input
                                && (!signature.effects.writes.is_empty()
                                    || signature
                                        .param_conventions
                                        .contains(&ast::ParamConvention::Mutate)))
                            || signature
                                .params
                                .iter()
                                .zip(&signature.param_conventions)
                                .any(|((_, ty), convention)| {
                                    *convention == ast::ParamConvention::Mutate
                                        && typed
                                            .type_registry
                                            .integer_validity_for_type(
                                                ty.without_reference_wrappers(),
                                            )
                                            .is_some()
                                })
                    }) =>
            {
                return Vec::new();
            }
            _ => {}
        }
    }
    if !dependencies.iter().all(|name| {
        let root = name.as_str().split('.').next().unwrap_or(name.as_str());
        if root == "$result" {
            return exits.iter().all(|exit| {
                exit.as_ref()
                    .unwrap()
                    .types
                    .get(&internment::Intern::from_ref("$result"))
                    == Some(&bind.return_type)
            });
        }
        bind.params.iter().any(|(parameter, ty)| {
            parameter.as_str() == root
                && exits
                    .iter()
                    .all(|exit| exit.as_ref().unwrap().types.get(parameter) == Some(ty))
        })
    }) {
        return Vec::new();
    }
    vec![proof.proposition.clone()]
}
