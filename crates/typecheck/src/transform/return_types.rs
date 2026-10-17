use crate::typed::DefId;
use crate::typed::{BindBody, ExprId, TypedExprKind, TypedFileAst};
use ast::integer::{IntegerDomain, IntegerRepresentationRule, IntegerValidity};
use ast::ty::Ty;
use diagnostic::DiagnosticCode;
use diagnostic::code::{PropositionCode, ResultCode, TypeCode};
use std::collections::{HashMap, HashSet};

pub(super) fn has_no_results(ty: &Ty) -> bool {
    matches!(ty, Ty::Unit) || ty.sequence_len() == Some(0)
}

pub(super) fn validate_unit_fallthrough(
    typed: &mut TypedFileAst,
    bind: &ast::Bind,
    expression: ExprId,
) {
    let body = BindBody::Body {
        exprs: vec![expression],
        ret: None,
    };
    if crate::analysis::return_expressions(typed, &body)
        .into_iter()
        .any(|value| {
            typed
                .exprs
                .ty_of(value)
                .is_some_and(|ty| !has_no_results(ty))
        })
    {
        typed.declaration_flaws.push((
            bind.name_span,
            // type-return-mismatch
            diagnostic::Diagnostic::new(
                ResultCode::ReturnMismatch,
                "functions returning values cannot fall through without a result",
            )
            .at_span_id(bind.name_span, &typed.span_table),
        ));
    }
}

pub(super) fn order_return_dependencies(definitions: &mut Vec<DefId>, ast: &ast::FileAst) {
    fn collect(expression: &ast::Expr, dependencies: &mut Vec<DefId>) {
        if let ast::Expr::FnCall(call) = expression {
            let target = call.definition.unwrap_or_else(|| {
                DefId::named(internment::Intern::new(call.path.to_qualified_string()))
            });
            if !dependencies.contains(&target) {
                dependencies.push(target);
            }
        }
        let _ = ast::folder::walk_expr_children(expression, &mut |_, child| {
            collect(child, dependencies);
            std::ops::ControlFlow::Continue(())
        });
    }
    let graph = definitions
        .iter()
        .map(|definition| {
            let mut dependencies = Vec::new();
            if let Some(bind) = ast.defs.get(definition) {
                match &bind.value {
                    ast::BindValue::Expr(expression) => {
                        collect(&expression.value, &mut dependencies)
                    }
                    ast::BindValue::Body { exprs, ret } => {
                        for expression in exprs.iter().chain(&ret.values) {
                            collect(&expression.value, &mut dependencies);
                        }
                    }
                    ast::BindValue::Unassigned => {}
                }
            }
            dependencies.retain(|target| ast.defs.get(target).is_some_and(infers_return_type));
            (*definition, dependencies)
        })
        .collect::<HashMap<_, _>>();
    let mut ordered = Vec::with_capacity(definitions.len());
    let mut visited = HashSet::new();
    for definition in definitions.iter().copied() {
        if !visited.insert(definition) {
            continue;
        }
        let mut pending = vec![(definition, 0)];
        while let Some((definition, next_dependency)) = pending.last_mut() {
            let dependencies = graph
                .get(definition)
                .expect("return dependency traversal only schedules graph members");
            if let Some(dependency) = dependencies.get(*next_dependency) {
                *next_dependency += 1;
                if visited.insert(*dependency) && graph.contains_key(dependency) {
                    pending.push((*dependency, 0));
                }
            } else {
                ordered.push(*definition);
                pending.pop();
            }
        }
    }
    *definitions = ordered;
}

pub(super) fn infers_return_type(bind: &ast::Bind) -> bool {
    bind.return_tag.is_none()
        && bind.return_refinement.is_none()
        && bind.return_type_name.is_none()
        && bind.type_annotation.is_none()
        && bind.named_results.is_empty()
        && bind.anonymous_result_alternatives.is_empty()
}

pub(super) fn validate_proposition_returns(
    typed: &mut TypedFileAst,
    bind: &ast::Bind,
    body: &BindBody,
) {
    let returns = crate::analysis::return_expressions(typed, body);
    let types = returns
        .iter()
        .filter_map(|expression| typed.exprs.ty_of(*expression))
        .collect::<Vec<_>>();
    let Some(expected) = types.iter().find(|ty| matches!(ty, Ty::Proposition(_))) else {
        return;
    };
    if types.iter().any(|actual| actual != expected) {
        typed.declaration_flaws.push((
            bind.name_span,
            // type-proposition-mismatch
            diagnostic::Diagnostic::new(
                PropositionCode::Mismatch,
                "all return paths must describe the same proposition",
            )
            .at_span_id(bind.name_span, &typed.span_table),
        ));
    }
}

pub(super) fn infer_integer_return_type(
    typed: &mut TypedFileAst,
    bind: &ast::Bind,
    body: &BindBody,
) -> Option<Ty> {
    let expressions = crate::analysis::return_expressions(typed, body);
    let types = expressions
        .into_iter()
        .map(|expression| expression_result_type(typed, expression))
        .collect::<Option<Vec<_>>>()?;
    let joined = match join_results(&types, &typed.type_registry) {
        Ok(joined) => joined?,
        Err(code) => {
            typed.declaration_flaws.push((
                bind.name_span,
                diagnostic::Diagnostic::new(
                    code,
                    "integer return paths require compatible types and valid values",
                )
                .at_span_id(bind.name_span, &typed.span_table),
            ));
            return None;
        }
    };
    if !contains_integer(&joined, &typed.type_registry) {
        return None;
    }
    if matches!(body, BindBody::Body { ret: None, .. }) {
        typed.declaration_flaws.push((
            bind.name_span,
            // type-return-mismatch
            diagnostic::Diagnostic::new(
                ResultCode::ReturnMismatch,
                "integer return paths require a value on the final return",
            )
            .at_span_id(bind.name_span, &typed.span_table),
        ));
    }
    Some(joined)
}

fn contains_integer(ty: &Ty, registry: &crate::TypeRegistry) -> bool {
    match ty {
        Ty::AnonymousInteger { .. } => true,
        Ty::Sequence(fields) => fields
            .iter()
            .any(|field| contains_integer(&field.ty, registry)),
        Ty::Named { .. } => registry.integer_validity_for_type(ty).is_some(),
        _ => false,
    }
}

fn expression_result_type(typed: &TypedFileAst, expression: ExprId) -> Option<Ty> {
    let ty = typed.exprs.ty_of(expression)?;
    match ty {
        Ty::AnonymousInteger { validity } => Some(Ty::AnonymousInteger {
            validity: IntegerValidity::new(
                typed
                    .exprs
                    .integer_knowledge_of(expression)
                    .and_then(Option::as_ref)
                    .and_then(super::place::knowledge_domain)
                    .unwrap_or_else(|| validity.domain().clone()),
            ),
        }),
        Ty::Sequence(fields) => {
            let mut fields = fields.clone();
            if let Some(TypedExprKind::ResultRow(values)) = typed.exprs.kind_of(expression) {
                assert_eq!(
                    fields.len(),
                    values.len(),
                    "result row {expression:?} type and values must have equal arity"
                );
                for (field, value) in fields.iter_mut().zip(values) {
                    *field.ty = expression_result_type(typed, *value)?;
                }
            }
            Some(Ty::Sequence(fields))
        }
        _ => Some(ty.clone()),
    }
}

fn join_results(
    types: &[Ty],
    registry: &crate::TypeRegistry,
) -> Result<Option<Ty>, DiagnosticCode> {
    let Some(first) = types.first() else {
        return Ok(None);
    };
    if types.iter().all(|ty| ty == first) {
        return Ok(Some(first.clone()));
    }
    if types
        .iter()
        .all(|ty| ty.anonymous_integer_validity().is_some())
    {
        return Ok(Some(Ty::AnonymousInteger {
            validity: IntegerValidity::new(IntegerDomain::union(types.iter().filter_map(|ty| {
                ty.anonymous_integer_validity()
                    .map(|validity| validity.domain().clone())
            }))),
        }));
    }
    if types.iter().all(|ty| {
        matches!(ty, Ty::Named { .. } | Ty::AnonymousInteger { .. })
            && registry.integer_validity_for_type(ty).is_some()
    }) {
        let named = types
            .iter()
            .find(|ty| ty.anonymous_integer_validity().is_none())
            .unwrap();
        let expected = registry.integer_validity_for_type(named).unwrap();
        for ty in types {
            if let Some(actual) = ty.anonymous_integer_validity() {
                let proof = actual.domain().prove_containment(expected.domain());
                if let Some(code) = proof.diagnostic_code()
                    && !(proof == ast::integer::ContainmentProof::Unknown
                        && registry.is_target_dependent_validity(expected))
                {
                    return Err(code);
                }
            } else if ty != named {
                return Err(DiagnosticCode::Type(TypeCode::Result(
                    ResultCode::ReturnMismatch,
                )));
            }
        }
        return Ok(Some(named.clone()));
    }
    let Ty::Sequence(first) = first else {
        return Ok(None);
    };
    if !types.iter().all(|ty| matches!(ty, Ty::Sequence(fields) if fields.len() == first.len() && fields.iter().zip(first).all(|(field, expected)| field.name == expected.name))) {
        return Ok(None);
    }
    let mut fields = first.clone();
    for (index, field) in fields.iter_mut().enumerate() {
        let column = types
            .iter()
            .filter_map(|ty| match ty {
                Ty::Sequence(fields) => Some((*fields[index].ty).clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let Some(joined) = join_results(&column, registry)? else {
            return Ok(None);
        };
        *field.ty = joined;
    }
    Ok(Some(Ty::Sequence(fields)))
}

pub(super) fn validate_integer_return_representation(
    typed: &mut TypedFileAst,
    bind: &ast::Bind,
    ty: &Ty,
) {
    match ty {
        Ty::AnonymousInteger { validity }
            if !typed.type_registry.is_target_dependent_validity(validity) =>
        {
            if let Err(error) = validity
                .resolve_runtime_representation(&IntegerRepresentationRule::InferFromValidity)
            {
                typed.declaration_flaws.push((bind.name_span, diagnostic::Diagnostic::new(error.diagnostic_code(), "inferred integer return requires finite bounds and a supported runtime width").at_span_id(bind.name_span, &typed.span_table)));
            }
        }
        Ty::Sequence(fields) => {
            for field in fields {
                validate_integer_return_representation(typed, bind, &field.ty);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use internment::Intern;
    use parser::query::SourceParseExt;

    #[test]
    fn result_row_arity_mismatches_are_internal_errors() {
        let parsed = "choose:\n    return 1, 2\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let typed = crate::transform::transform_file(parsed.ast, crate::FileId(0));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let index = typed
            .exprs
            .kind
            .iter()
            .position(|kind| matches!(kind, TypedExprKind::ResultRow(_)))
            .expect("result row fixture");
        assert!(expression_result_type(&typed, ExprId(index as u32)).is_some());
        for expected_count in [1, 3] {
            let mut changed = typed.clone();
            let Ty::Sequence(fields) = &mut changed.exprs.ty[index] else {
                panic!("result row type")
            };
            fields.resize(expected_count, fields[0].clone());
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    expression_result_type(&changed, ExprId(index as u32))
                }))
                .is_err(),
                "mismatched row length {expected_count}"
            );
        }
    }

    #[test]
    fn return_dependencies_preserve_order_and_terminate_on_cycles() {
        let parsed = "first: second\nsecond: first\nindependent: 1\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let id = |name| DefId::named(Intern::from_ref(name));
        let mut definitions = vec![id("first"), id("second"), id("independent")];
        order_return_dependencies(&mut definitions, &parsed.ast);
        assert_eq!(
            definitions,
            vec![id("second"), id("first"), id("independent")]
        );
    }

    #[test]
    fn long_return_dependency_chains_fit_a_small_stack() {
        const CHILD: &str = "GIN_RETURN_DEPENDENCIES_SMALL_STACK";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "transform::return_types::tests::long_return_dependency_chains_fit_a_small_stack", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let count = 2000;
        let source = (0..count)
            .map(|index| {
                if index + 1 == count {
                    format!("f{index}: 1\n")
                } else {
                    format!("f{index}: f{}(0)\n", index + 1)
                }
            })
            .collect::<String>();
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut definitions = (0..count)
            .map(|index| DefId::named(Intern::new(format!("f{index}"))))
            .collect::<Vec<_>>();
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || {
                order_return_dependencies(&mut definitions, &parsed.ast);
                assert_eq!(definitions.len(), count);
                assert_eq!(
                    definitions.first(),
                    Some(&DefId::named(Intern::new(format!("f{}", count - 1))))
                );
                assert_eq!(
                    definitions.last(),
                    Some(&DefId::named(Intern::from_ref("f0")))
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

pub(super) fn same_union_layout(
    registry: &crate::TypeRegistry,
    actual: &Ty,
    expected: &Ty,
) -> bool {
    match (
        registry.resolved_definition_for_type(actual),
        registry.resolved_definition_for_type(expected),
    ) {
        (
            Ty::Union {
                name: actual_name,
                variants: actual,
                literal_values: actual_literals,
                ..
            },
            Ty::Union {
                name: expected_name,
                variants: expected,
                literal_values: expected_literals,
                ..
            },
        ) => {
            (actual_name.as_str().starts_with('(') || expected_name.as_str().starts_with('('))
                && actual == expected
                && actual_literals == expected_literals
        }
        _ => false,
    }
}
