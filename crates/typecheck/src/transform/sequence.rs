use ast::Ty;
use ast::prelude::*;
use diagnostic::Diagnostic;
use diagnostic::code::SequenceCode;
use internment::Intern;

use crate::typed::{
    ExprId, SequenceTraversal, TypedExprKind, TypedFileAst, TypedLoop, TypedLoopKind,
};

use super::lower_exprs::{
    ExprLowerScope, FlowState, append_derived_expr, join_loop_place_versions, join_place_versions,
    lower_typed_expr,
};
use super::lower_kind::root_place_versions_since;

fn local(name: Intern<String>, span: SpanId) -> Typed<Expr> {
    Typed::infer(
        Expr::FnCall(ast::expr::FnCall {
            syntax: ast::CallSyntax::Path,
            definition: None,
            path: Spanned::new(ModPath::new(name, Vec::new()), span),
            args: None,
        }),
        span,
    )
}

fn binding(name: Intern<String>, value: Typed<Expr>, span: SpanId) -> Typed<Expr> {
    Typed::infer(
        Expr::Bind(Box::new(ast::Bind::new(
            name,
            span,
            BindValue::Expr(Box::new(value)),
        ))),
        span,
    )
}

fn lower_body(
    typed: &mut TypedFileAst,
    expressions: &[Typed<Expr>],
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> Vec<ExprId> {
    expressions
        .iter()
        .map(|expression| lower_typed_expr(typed, expression, &scope.statement(), env))
        .collect()
}

pub(super) fn lower_for(
    typed: &mut TypedFileAst,
    for_in: &ast::ForInLoop,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> TypedExprKind {
    let incoming = env.clone();
    let mut sequence_env = incoming.clone();
    let source_name = Intern::new(format!("%sequence_{}", typed.exprs.kind.len()));
    let source = binding(
        source_name,
        for_in.iter.as_ref().clone(),
        for_in.iter.span_id,
    );
    let source = lower_typed_expr(typed, &source, &scope.child(), &mut sequence_env);
    let TypedExprKind::Bind { body: value, .. } = typed.exprs.kind[source.index()] else {
        unreachable!();
    };
    let variable = match &for_in.pat.value {
        Expr::Bind(bind) => bind.name,
        Expr::FnCall(call) => call.path.root,
        _ => unreachable!(),
    };

    let (iterable, traversal, place_phis) = if let TypedExprKind::Range { start, end } =
        typed.exprs.kind[value.index()]
    {
        let mut body_env = incoming.clone();
        super::lower_exprs::prepare_loop_place_versions(typed, &mut body_env);
        body_env.places.remove(&variable);
        body_env.place_versions.remove(&variable);
        body_env.constants.remove(&variable);
        body_env.const_values.remove(&variable);
        body_env.target_groups.remove(&variable);
        body_env.result_target_groups.remove(&variable);
        body_env.result_evidence.remove(&variable);
        body_env.projected_result_evidence.remove(&variable);
        body_env.result_names.remove(&variable);
        body_env.local_callables.remove(&variable);
        let hull = |expression: ExprId| {
            typed
                .type_registry
                .integer_validity_for_type(&typed.exprs.ty[expression.index()])
                .and_then(|validity| validity.domain().storage_hull())
        };
        let item_type = match (hull(start), hull(end)) {
            (Some(start), Some(end)) => Ty::AnonymousInteger {
                validity: ast::integer::IntegerValidity::new(
                    ast::integer::IntegerDomain::bounded(
                        start.min().min(end.min()),
                        start.max().max(end.max()),
                    )
                    .unwrap(),
                ),
            },
            _ => Ty::i64(),
        };
        let counter = typed.add_place(crate::typed::TypedPlace {
            binder: ast::BinderId::new(
                typed.file_id.0,
                ast::BinderOwner::Expression(typed.exprs.kind.len() as u32),
            ),
            name: variable,
            parent: None,
            projection: None,
            mutable: false,
            explicit_contract: true,
            ty: item_type.clone(),
        });
        let version = typed.add_place_version(
            counter,
            crate::typed::PlaceVersionOrigin::Parameter,
            item_type.clone(),
            None,
        );
        body_env.places.insert(variable, counter);
        body_env.place_versions.insert(variable, version);
        body_env.types.insert(variable, item_type);
        let item = binding(
            variable,
            local(variable, for_in.pat.span_id),
            for_in.pat.span_id,
        );
        let item = lower_typed_expr(typed, &item, &scope.child(), &mut body_env);
        let mut stmts = vec![item];
        stmts.extend(lower_body(typed, &for_in.exprs, scope, &mut body_env));
        let first_phi = typed.place_versions.len();
        join_loop_place_versions(typed, env, &incoming, &body_env);
        let place_phis = root_place_versions_since(typed, first_phi, |origin| {
            matches!(origin, crate::typed::PlaceVersionOrigin::LoopPhi { .. })
        });
        (
            value,
            SequenceTraversal::IntegerRange {
                variable,
                start,
                end,
                stmts,
            },
            place_phis,
        )
    } else if let Some((permission, length)) =
        runtime_sequence_length(typed, value, scope, &mut sequence_env)
    {
        let index = Intern::new(format!("%index_{}", source.index()));
        let index_type = Ty::AnonymousInteger {
            validity: typed
                .type_registry
                .integer_validity_for_type(&typed.exprs.ty[length.index()])
                .unwrap()
                .clone(),
        };
        let mut body_env = sequence_env.clone();
        super::lower_exprs::prepare_loop_place_versions(typed, &mut body_env);
        body_env.types.insert(index, index_type);
        let element = Typed::infer(
            Expr::BufGet {
                buf: Box::new(local(source_name, for_in.iter.span_id)),
                index: Box::new(local(index, for_in.iter.span_id)),
            },
            for_in.pat.span_id,
        );
        let element = Typed::infer(
            Expr::Ref {
                inner: Box::new(element),
                mutable: permission == crate::type_registry::ReferencePermission::Mutate
                    && crate::ty::reference_semantics_for_type(
                        &typed.exprs.ty[value.index()],
                        Some(&typed.type_registry),
                    )
                    .is_some_and(|reference| {
                        reference.permission == crate::type_registry::ReferencePermission::Mutate
                    }),
                group: None,
            },
            for_in.pat.span_id,
        );
        let item = binding(variable, element, for_in.pat.span_id);
        let item = lower_typed_expr(typed, &item, &scope.child(), &mut body_env);
        let mut stmts = vec![item];
        stmts.extend(lower_body(typed, &for_in.exprs, scope, &mut body_env));
        if permission == crate::type_registry::ReferencePermission::Mutate {
            env.invalidate_mutable_constants();
        }
        let first_phi = typed.place_versions.len();
        join_loop_place_versions(typed, env, &sequence_env, &body_env);
        let place_phis = root_place_versions_since(typed, first_phi, |origin| {
            matches!(origin, crate::typed::PlaceVersionOrigin::LoopPhi { .. })
        });
        (
            source,
            SequenceTraversal::RuntimeView {
                index,
                length,
                stmts,
            },
            place_phis,
        )
    } else {
        let source_type = typed.exprs.ty[source.index()].clone();
        let source_type =
            crate::ty::reference_pointee_for_type(&source_type, Some(&typed.type_registry))
                .unwrap_or(source_type);
        let source_type = typed
            .type_registry
            .resolved_definition_for_type(&source_type);
        let length = source_type.sequence_len();
        match length {
            Some(length) => {
                let mut iterations = Vec::with_capacity(length);
                for index in 0..length {
                    let mut body_env = sequence_env.clone();
                    let item = Typed::infer(
                        Expr::AggregateGet {
                            base: Box::new(local(source_name, for_in.iter.span_id)),
                            index,
                        },
                        for_in.pat.span_id,
                    );
                    let item = binding(variable, item, for_in.pat.span_id);
                    let item = lower_typed_expr(typed, &item, &scope.child(), &mut body_env);
                    let mut stmts = vec![item];
                    stmts.extend(lower_body(typed, &for_in.exprs, scope, &mut body_env));
                    let before = sequence_env.clone();
                    join_place_versions(typed, &mut sequence_env, &before, &[body_env], false);
                    iterations.push(stmts);
                }
                join_place_versions(typed, env, &incoming, &[sequence_env], false);
                (source, SequenceTraversal::Fixed { iterations }, Vec::new())
            }
            None => {
                typed.exprs.facts.semantic_facts.flaws[value.index()].push(
                    // type-for-requires-sequence
                    Diagnostic::new(
                        SequenceCode::IterationRequiresSequence,
                        "for source must be a sequence or an integer range",
                    )
                    .at_span_id(for_in.iter.span_id, &typed.span_table),
                );
                (source, SequenceTraversal::Unresolved, Vec::new())
            }
        }
    };

    TypedExprKind::Loop(TypedLoop {
        kind: TypedLoopKind::ForIn {
            iterable,
            traversal,
        },
        place_phis,
        keyword_span: for_in.keyword_span,
    })
}

fn runtime_sequence_length(
    typed: &mut TypedFileAst,
    value: ExprId,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> Option<(crate::type_registry::ReferencePermission, ExprId)> {
    let ty = typed.exprs.ty[value.index()].clone();
    let reference = crate::ty::reference_semantics_for_type(&ty, Some(&typed.type_registry))?;
    let Ty::Array { size, .. } = typed
        .type_registry
        .resolved_definition_for_type(&reference.pointee)
    else {
        return None;
    };
    if size.as_const_int().is_some() {
        return None;
    }
    let length = lower_length_term(typed, value, &size, scope, env)?;
    Some((reference.permission, length))
}

fn lower_length_term(
    typed: &mut TypedFileAst,
    source: ExprId,
    term: &ast::NormalExpr,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> Option<ExprId> {
    let ast::NormalExpr::Var(name) = term else {
        return None;
    };
    let mut parts = name.as_str().split(".$field");
    let root: Intern<String> = Intern::from_ref(parts.next()?);
    let mut base = if let Some(call) = root
        .as_str()
        .strip_prefix("$call")
        .and_then(|id| id.parse::<u32>().ok())
    {
        append_derived_expr(
            typed,
            source,
            TypedExprKind::FnCall {
                target: crate::typed::DefId::named(root),
                args: None,
                substituted_ty: None,
            },
            typed
                .exprs
                .ty
                .get(call as usize)?
                .without_reference_wrappers()
                .clone(),
        )
    } else {
        lower_typed_expr(
            typed,
            &local(root, typed.exprs.span[source.index()]),
            &scope.child(),
            env,
        )
    };
    for part in parts {
        let index = part.parse::<usize>().ok()?;
        let ty = typed.exprs.ty[base.index()].without_reference_wrappers();
        let Ty::Record { fields, .. } = typed.type_registry.resolved_definition_for_type(ty) else {
            return None;
        };
        base = append_derived_expr(
            typed,
            base,
            TypedExprKind::AggregateGet { base, index },
            *fields.get(index)?.1.clone(),
        );
    }
    Some(base)
}
