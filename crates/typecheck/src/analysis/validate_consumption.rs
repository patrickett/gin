use ast::ParamConvention;
use diagnostic::code::{CallCode, ReferenceCode, TypeCode};
use diagnostic::{Diagnostic, DiagnosticCode, RelatedSpan};

use crate::typed::{
    ExprId, GroupId, ReferenceSource, ReferenceTargetGroup, ReferenceTargetSet, ReferenceValue,
    TypedExprKind, TypedFileAst,
};

enum ContainedReturnTargets {
    NoReferences,
    Unknown,
    Known(ReferenceTargetSet),
}

fn contained_return_targets(
    shape: &ReferenceValue,
    value: &ReferenceValue,
) -> ContainedReturnTargets {
    let mut targets = ReferenceTargetSet::default();
    let mut has_references = false;
    let mut unknown = false;
    fn collect(
        shape: &ReferenceValue,
        value: &ReferenceValue,
        targets: &mut ReferenceTargetSet,
        has_references: &mut bool,
        unknown: &mut bool,
    ) {
        if !shape.sources.is_empty() {
            *has_references = true;
            *unknown |= value.sources.is_empty();
            for source in &value.sources {
                match source {
                    ReferenceSource::Target(target) => {
                        targets.extend(&ReferenceTargetSet::singleton(target.clone()))
                    }
                    ReferenceSource::Argument(argument)
                    | ReferenceSource::ArgumentElements(argument) => {
                        targets.extend(&ReferenceTargetSet::singleton(ReferenceTargetGroup::Param(
                            GroupId(argument.parameter as u32),
                        )))
                    }
                    ReferenceSource::Unknown => *unknown = true,
                }
            }
            return;
        }
        if let Some(element) = &shape.array_element {
            if let Some(value) = &value.array_element {
                collect(element, value, targets, has_references, unknown);
            }
            if value.fields.is_empty() && value.array_element.is_none() {
                collect(
                    element,
                    &ReferenceValue::default(),
                    targets,
                    has_references,
                    unknown,
                );
            }
            for field in &value.fields {
                collect(element, field, targets, has_references, unknown);
            }
        }
        for (index, field) in shape.fields.iter().enumerate() {
            collect(
                field,
                value
                    .fields
                    .get(index)
                    .unwrap_or(&ReferenceValue::default()),
                targets,
                has_references,
                unknown,
            );
        }
    }
    collect(
        shape,
        value,
        &mut targets,
        &mut has_references,
        &mut unknown,
    );
    if !has_references {
        return ContainedReturnTargets::NoReferences;
    }
    if unknown && !targets.iter().any(|target| target.root_local().is_some()) {
        return ContainedReturnTargets::Unknown;
    }
    ContainedReturnTargets::Known(targets)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgumentMode {
    Own,
    Observe,
    Mutate,
}

fn argument_mode(typed: &TypedFileAst, expr_id: ExprId) -> ArgumentMode {
    match typed.exprs.kind_of(expr_id) {
        Some(TypedExprKind::Ref(_)) => match typed.exprs.ty_of(expr_id) {
            Some(ty) if ty.is_mutable_ref() => ArgumentMode::Mutate,
            _ => ArgumentMode::Observe,
        },
        _ => ArgumentMode::Own,
    }
}

fn is_addressable_place(typed: &TypedFileAst, expr_id: ExprId) -> bool {
    match typed.exprs.kind_of(expr_id) {
        Some(TypedExprKind::FnCall { args, .. }) => {
            args.as_ref().is_none_or(|args| args.is_empty())
        }
        Some(TypedExprKind::SelfRef { .. }) => typed.exprs.place_of(expr_id).flatten().is_some(),
        Some(TypedExprKind::AggregateGet { base, .. }) => is_addressable_place(typed, *base),
        Some(TypedExprKind::SliceGet { buf, .. } | TypedExprKind::BufGet { buf, .. }) => {
            is_addressable_place(typed, *buf)
        }
        Some(TypedExprKind::Deref(_)) => true,
        _ => false,
    }
}

fn expected_mode(convention: ParamConvention) -> ArgumentMode {
    match convention {
        ParamConvention::Terminal => ArgumentMode::Own,
        ParamConvention::Own => ArgumentMode::Own,
        ParamConvention::Observe => ArgumentMode::Observe,
        ParamConvention::Mutate => ArgumentMode::Mutate,
    }
}

fn mode_name(mode: ArgumentMode) -> &'static str {
    match mode {
        ArgumentMode::Own => "owned",
        ArgumentMode::Observe => "ref",
        ArgumentMode::Mutate => "mut",
    }
}

fn validate_reference_returns(typed: &mut TypedFileAst) {
    let mut checks = Vec::new();
    for bind in typed.defs.values() {
        let signature = crate::typed::TypedCallableSignature::from(bind);
        let returns = crate::analysis::return_expressions(typed, &bind.body);
        for expr in returns {
            for (index, result) in signature.results.iter().enumerate() {
                if crate::ty::reference_semantics_for_type(&result.ty, Some(&typed.type_registry))
                    .is_none()
                {
                    let references = typed.exprs.references_of(expr.index());
                    let references = if matches!(bind.return_type, ast::Ty::Sequence(_)) {
                        references.fields.get(index).cloned().unwrap_or_default()
                    } else {
                        references.clone()
                    };
                    let shape = crate::transform::references::reference_shape(typed, &result.ty);
                    match contained_return_targets(&shape, &references) {
                        ContainedReturnTargets::NoReferences => {}
                        ContainedReturnTargets::Unknown => checks.push((expr, None, None, true)),
                        ContainedReturnTargets::Known(actual) => {
                            checks.push((expr, None, Some(actual), true))
                        }
                    }
                    continue;
                }
                let (site, actual) = match typed.exprs.kind_of(expr) {
                    Some(TypedExprKind::ResultRow(fields)) => {
                        let Some(field) = fields.get(index) else {
                            continue;
                        };
                        (
                            *field,
                            typed.exprs.target_group_of(*field).cloned().flatten(),
                        )
                    }
                    _ if matches!(bind.return_type, ast::Ty::Sequence(_)) => (
                        expr,
                        typed
                            .exprs
                            .result_target_groups_of(expr)
                            .and_then(|groups| groups.get(index))
                            .cloned()
                            .flatten(),
                    ),
                    _ => (expr, typed.exprs.target_group_of(expr).cloned().flatten()),
                };
                let references = typed.exprs.references_of(site.index());
                let actual = if references.sources.iter().any(|source| matches!(source, ReferenceSource::Target(target) if target.is_static()))
                {
                    references.targets_in_definition(bind, &typed.type_registry).or(actual)
                } else {
                    actual
                };
                let expected = bind.result_groups.get(index).copied().flatten();
                checks.push((site, expected, actual, false));
            }
        }
    }

    for (expr, expected, actual, contained) in checks {
        let (code, message) = match actual.as_ref() {
            Some(targets)
                if targets.iter().next().is_some()
                    && targets.iter().all(|target| target.is_static()) =>
            {
                continue;
            }
            Some(targets)
                if (contained || expected.is_some())
                    && targets.iter().next().is_some()
                    && targets.iter().all(|target| {
                        target.is_static()
                            || if contained {
                                target.root_param().is_some()
                            } else {
                                target.root_param() == expected
                            }
                    }) =>
            {
                continue;
            }
            Some(targets) if targets.iter().any(|target| target.root_param().is_none()) => (
                // type-reference-return-local-target
                DiagnosticCode::Type(TypeCode::Reference(ReferenceCode::ReturnLocalTarget)),
                "cannot return a reference to local storage",
            ),
            Some(targets) if expected.is_some() && targets.iter().next().is_some() => (
                // type-reference-return-wrong-target-group
                DiagnosticCode::Type(TypeCode::Reference(ReferenceCode::ReturnWrongTargetGroup)),
                "returned reference comes from the wrong target group",
            ),
            _ => (
                // type-reference-return-unknown-target
                DiagnosticCode::Type(TypeCode::Reference(ReferenceCode::ReturnUnknownTarget)),
                "cannot prove the returned reference outlives the call",
            ),
        };
        if typed.exprs.facts.semantic_facts.flaws[expr.as_usize()]
            .iter()
            .any(|flaw| flaw.code == code)
        {
            continue;
        }
        let mut diagnostic = Diagnostic::new(code, message);
        if let Some(span) = typed.exprs.span_of(expr) {
            diagnostic = diagnostic.at_span_id(span, &typed.span_table);
        }
        diagnostic = match code {
            DiagnosticCode::Type(TypeCode::Reference(ReferenceCode::ReturnLocalTarget)) => diagnostic
                .with_help_on_span("this reference escapes the local storage's lifetime")
                .with_help("return the value by ownership instead, or return a reference to caller-owned storage"),
            DiagnosticCode::Type(TypeCode::Reference(ReferenceCode::ReturnWrongTargetGroup)) => diagnostic
                .with_help_on_span("this reference comes from a different input target group")
                .with_help("return a reference from the declared result target group"),
            _ => diagnostic
                .with_help_on_span("the lifetime of this reference cannot be established")
                .with_help("return an owned value, or a reference with a known input target group"),
        };
        if let Some(targets) = actual {
            let mut locals: Vec<_> = targets
                .iter()
                .filter_map(|target| target.root_local())
                .collect();
            locals.sort_by_key(|place| place.0);
            locals.dedup();
            for local in locals {
                let Some(place) = typed.places.get(local.0 as usize) else {
                    continue;
                };
                let span = typed
                    .defs
                    .values()
                    .find(|bind| bind.dependent_binder == place.binder)
                    .map(|bind| bind.name_span)
                    .or_else(|| {
                        typed.exprs.place_entries().find_map(|(expr, candidate)| {
                            (candidate == Some(local)
                                && matches!(
                                    typed.exprs.kind_of(expr),
                                    Some(TypedExprKind::Bind { .. })
                                ))
                            .then(|| typed.exprs.span_of(expr))
                            .flatten()
                        })
                    });
                if let Some(span) = span {
                    diagnostic.related_mut().push(RelatedSpan {
                        span: typed.span_table.get(span),
                        label: format!(
                            "local `{}` is declared here; its storage does not outlive this call",
                            place.name
                        ),
                    });
                }
            }
        }
        typed.exprs.facts.semantic_facts.flaws[expr.index()].push(diagnostic);
    }
}

fn validate_reference_writes(typed: &mut TypedFileAst) {
    let mut escaping = Vec::new();
    for (id, bind) in &typed.defs {
        let mut local = false;
        for value in bind
            .references
            .writes
            .values()
            .chain(bind.references.unknown_slot_writes.values())
        {
            value.visit(&mut |value| {
                local |= value.sources.iter().any(|source| {
                    matches!(source, ReferenceSource::Target(target) if target.root_local().is_some())
                });
            });
        }
        if local {
            escaping.push(*id);
        }
    }
    for id in escaping {
        let bind = typed
            .defs
            .get_mut(&id)
            .expect("a checked reference-write definition remains in the definition map");
        // type-reference-store-local-target
        let code = DiagnosticCode::Type(TypeCode::Reference(ReferenceCode::StoreLocalTarget));
        if !bind.flaws.iter().any(|flaw| flaw.code == code) {
            bind.push_flaw(
                Diagnostic::new(
                    code,
                    "cannot store a reference to local storage in caller-owned storage",
                )
                .at_span_id(bind.name_span, &typed.span_table),
            );
        }
    }
}

pub fn stage_validate_consumption(typed: &mut TypedFileAst) {
    let _profile = compiler_profile::span(compiler_profile::Phase::Consumption);
    validate_reference_returns(typed);
    validate_reference_writes(typed);

    for idx in 0..typed.exprs.kind.len() {
        // Clone `args` out of the arena: the borrow of `typed.exprs` cannot
        // outlive the destination writes below (the arena is copy-on-write).
        let (target, args) = match &typed.exprs.kind[idx] {
            TypedExprKind::FnCall {
                target,
                args: Some(args),
                ..
            } => (*target, args.clone()),
            _ => continue,
        };
        let parameters: Option<Vec<_>> = if let Some(definition) = typed
            .local_callable_calls
            .get(&crate::typed::ExprId(idx as u32))
        {
            typed
                .local_callable_parameters
                .get(definition)
                .map(|parameters| {
                    parameters
                        .iter()
                        .map(|parameter| (parameter.name, parameter.convention))
                        .collect()
                })
        } else {
            typed.defs.get(&target).map(|bind| {
                bind.params
                    .iter()
                    .zip(&bind.param_conventions)
                    .map(|((name, _), convention)| (*name, *convention))
                    .collect()
            })
        };
        let Some(parameters) = parameters else {
            continue;
        };
        for (param_idx, (param_name, convention)) in parameters.into_iter().enumerate() {
            let Some(arg_id) = args.get(param_idx) else {
                continue;
            };
            let actual = argument_mode(typed, *arg_id);
            let expected = expected_mode(convention);
            if actual == expected {
                if matches!(actual, ArgumentMode::Observe | ArgumentMode::Mutate) {
                    let temporary_ref = matches!(
                        typed.exprs.kind_of(*arg_id),
                        Some(TypedExprKind::Ref(inner)) if !is_addressable_place(typed, *inner)
                    );
                    if temporary_ref {
                        typed.exprs.facts.semantic_facts.flaws[idx].push(
                            // type-reference-to-temporary
                            Diagnostic::new(
                                ReferenceCode::ToTemporary,
                                format!(
                                    "{} argument for parameter `{}` must be an addressable place",
                                    mode_name(actual),
                                    param_name
                                ),
                            )
                            .with_arg("name", param_name.as_str().to_string())
                            .with_arg("mode", mode_name(actual)),
                        );
                    }
                }
                continue;
            }
            // type-argument-mode-mismatch
            let code = DiagnosticCode::Type(TypeCode::Call(CallCode::ArgumentModeMismatch));
            typed.exprs.facts.semantic_facts.flaws[idx].push(
                Diagnostic::new(
                    code,
                    format!(
                        "parameter `{}` expects {}, found {}",
                        param_name,
                        mode_name(expected),
                        mode_name(actual)
                    ),
                )
                .with_arg("name", param_name.as_str().to_string())
                .with_arg("expected", mode_name(expected))
                .with_arg("actual", mode_name(actual)),
            );
        }
    }
}
