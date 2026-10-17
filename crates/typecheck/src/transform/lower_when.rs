//! When-arm lowering helpers.
//!
//! Provides functions for lowering the arms of `when` expressions,
//! including pattern-local variable binding and compile-time constant
//! arm selection.

use ast::ConstValue;
use ast::prelude::*;
use diagnostic::code::{PatternCode, PropositionCode};

use crate::analysis::TyOwnershipExt;
use crate::subst::DepSubst;
use crate::ty::{Ty, reference_pointee_for_type};
use crate::typed::{
    ExprId, PlaceVersionId, ReferenceTargetGroup, ReferenceTargetSet, TypedExprKind, TypedFileAst,
};

use super::lower_exprs::lower_typed_expr;
use super::lower_exprs::{ExprLowerScope, FlowState, join_place_versions};

pub(super) fn parameter_proposition_evidence(
    typed: &mut TypedFileAst,
    ty: &Ty,
    span: SpanId,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> Option<crate::typed::ValueEvidence> {
    let Ty::Proposition(proposition) = ty.without_reference_wrappers() else {
        return None;
    };
    let ast::ProofProposition::Compare {
        left,
        relation,
        right,
    } = proposition.as_ref()
    else {
        return None;
    };
    let mut lower = |term| {
        let normal = crate::solver::proof_term_to_normal(
            term,
            &ast::NormalExpr::from(0),
            &Default::default(),
        )?;
        let source = Typed::infer(crate::const_value::normal_expr_to_expr(&normal), span);
        Some(lower_typed_expr(typed, &source, &scope.child(), env))
    };
    let lhs = lower(left)?;
    let rhs = lower(right)?;
    Some(crate::typed::ValueEvidence::Proposition(
        crate::typed::EvidenceProposition::IntegerComparison {
            lhs,
            rhs,
            comparison: (*relation).into(),
        },
    ))
}

pub(crate) fn lower_condition(
    typed: &mut TypedFileAst,
    condition: &ast::Condition,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> crate::typed::TypedCondition {
    match condition {
        ast::Condition::Proposition { subject } => {
            let mut subject = lower_typed_expr(typed, subject, &scope.child(), env);
            if let Ty::Ref { inner, .. } = &typed.exprs.ty[subject.index()]
                && matches!(inner.as_ref(), Ty::Proposition(_))
            {
                subject = super::lower_exprs::append_derived_expr(
                    typed,
                    subject,
                    TypedExprKind::Deref(subject),
                    inner.as_ref().clone(),
                );
            }
            if !matches!(typed.exprs.ty[subject.index()], Ty::Proposition(_)) {
                // type-condition-requires-proposition
                let diagnostic = diagnostic::Diagnostic::new(PropositionCode::ConditionRequired, "conditions require a proposition; use an explicit 'is' pattern for user-defined values")
                    .at_span_id(typed.exprs.span[subject.index()], &typed.span_table);
                typed.exprs.facts.semantic_facts.flaws[subject.index()].push(diagnostic);
            }
            crate::typed::TypedCondition::Proposition { subject }
        }
        ast::Condition::Compare {
            left,
            relation,
            right,
        } => {
            let (left, right) =
                super::lower_kind::lower_numeric_operands(typed, left, right, scope, env);
            for operand in [left, right] {
                if typed
                    .type_registry
                    .integer_validity_for_type(&typed.exprs.ty[operand.index()])
                    .is_none()
                {
                    // type-numeric-predicate-operands
                    let diagnostic = diagnostic::Diagnostic::new(
                        PropositionCode::NumericOperandsRequired,
                        "numeric predicates require integer operands; use a named comparison function for custom orderings",
                    ).at_span_id(typed.exprs.span[operand.index()], &typed.span_table);
                    typed.exprs.facts.semantic_facts.flaws[operand.index()].push(diagnostic);
                }
            }
            crate::typed::TypedCondition::Compare {
                left,
                relation: *relation,
                right,
            }
        }
        ast::Condition::Is { subject, pattern } => {
            let subject = lower_typed_expr(typed, subject, &scope.child(), env);
            let _ = super::lower_exprs::ensure_temporary_owner(typed, env, subject);
            if matches!(
                typed.exprs.ty[subject.index()].without_reference_wrappers(),
                Ty::Proposition(_)
            ) {
                // type-proposition-pattern
                let diagnostic = diagnostic::Diagnostic::new(
                    PatternCode::Proposition,
                    "propositions are tested directly; remove the 'is' pattern",
                )
                .at_span_id(pattern.span_id, &typed.span_table);
                typed.exprs.facts.semantic_facts.flaws[subject.index()].push(diagnostic);
            }
            crate::typed::TypedCondition::Is {
                subject,
                pattern: Box::new(resolve_range_pattern(typed, pattern, scope)),
            }
        }
        ast::Condition::Not(inner) => {
            crate::typed::TypedCondition::Not(Box::new(lower_condition(typed, inner, scope, env)))
        }
        ast::Condition::And(left, right) => {
            let left = lower_condition(typed, left, scope, env);
            let (mut guarded, _) = with_condition_locals(typed, env, &left, scope);
            refine_integer_locals_from_condition(typed, &left, true, &mut guarded);
            let right = lower_condition(typed, right, scope, &mut guarded);
            for (name, previous) in env.place_versions.clone() {
                if let Some(current) = guarded.place_versions.get(&name)
                    && *current != previous
                {
                    env.set_place_version(name, *current);
                    env.integer_domains.remove(&previous);
                    env.union_members.remove(&previous);
                    env.const_values.remove(&name);
                    env.result_evidence.remove(&name);
                    env.projected_result_evidence.remove(&name);
                    env.projected_versions.clear();
                }
            }
            crate::typed::TypedCondition::And(Box::new(left), Box::new(right))
        }
        ast::Condition::Or(left, right) => crate::typed::TypedCondition::Or(
            Box::new(lower_condition(typed, left, scope, env)),
            Box::new(lower_condition(typed, right, scope, env)),
        ),
    }
}

fn resolve_range_pattern(
    typed: &TypedFileAst,
    pattern: &ast::Spanned<Pattern>,
    scope: &ExprLowerScope<'_>,
) -> ast::Spanned<Pattern> {
    let mut pattern = pattern.clone();
    if let Pattern::InRange {
        bounds: ast::InRangeBounds::Tag(name),
        span,
    } = &pattern.value
        && let Some(hull) = scope
            .tag_types
            .get(name)
            .and_then(|ty| typed.type_registry.integer_validity_for_type(ty))
            .and_then(|validity| match validity.domain().predicate() {
                ast::integer::CanonicalPredicate::Hull(hull) => Some(hull),
                _ => None,
            })
    {
        pattern.value = Pattern::InRange {
            bounds: ast::InRangeBounds::Literal(hull.min(), hull.max()),
            span: *span,
        };
    }
    pattern
}

fn refine_union(
    typed: &TypedFileAst,
    env: &mut FlowState,
    subject: ExprId,
    pattern: &Pattern,
    taken: bool,
) {
    let Some(version) = typed.exprs.place_version_of(subject).flatten() else {
        return;
    };
    if !env
        .place_versions
        .values()
        .any(|current| *current == version)
    {
        return;
    }
    let name = match pattern {
        Pattern::Nominal(name, _) | Pattern::Generic { name, .. } => name,
        _ => return,
    };
    let Some(ty) = typed.exprs.ty_of(subject) else {
        return;
    };
    let Ty::Union { variants, .. } = typed.type_registry.resolved_definition_for_type(ty) else {
        return;
    };
    let mut remaining = variants
        .iter()
        .enumerate()
        .filter(|(_, variant)| (variant.name == *name) == taken);
    if let Some((index, variant)) = remaining.next()
        && remaining.next().is_none()
        && variant.kind == ast::ty::UnionVariantKind::Type
        && let Some((_, ty)) = variant.fields.first()
    {
        env.union_members.insert(version, (index, (**ty).clone()));
    }
}

fn refine_range(typed: &TypedFileAst, env: &mut FlowState, subject: ExprId, pattern: &Pattern) {
    let Some(version) = typed.exprs.place_version_of(subject).flatten() else {
        return;
    };
    if !env
        .place_versions
        .values()
        .chain(env.projected_versions.values())
        .any(|current| *current == version)
    {
        return;
    }
    let Pattern::InRange { bounds, .. } = pattern else {
        return;
    };
    let (min, max) = match bounds {
        ast::InRangeBounds::Literal(min, max) => (*min, *max),
        ast::InRangeBounds::Tag(name) => {
            let Some(hull) = typed
                .tag_types
                .get(&crate::typed::TagId(*name))
                .and_then(|ty| typed.type_registry.integer_validity_for_type(ty))
                .and_then(|validity| validity.domain().storage_hull().cloned())
            else {
                return;
            };
            (hull.min(), hull.max())
        }
        ast::InRangeBounds::LiteralToTag(..) => return,
    };
    let base = env.integer_domains.get(&version).cloned().or_else(|| {
        typed
            .exprs
            .ty_of(subject)
            .and_then(|ty| typed.type_registry.integer_validity_for_type(ty))
            .map(|validity| validity.domain().clone())
    });
    let Some(base) = base else {
        return;
    };
    let (min, max) = base.storage_hull().map_or((min, max), |hull| {
        (min.max(hull.min()), max.min(hull.max()))
    });
    let mut domain = base.refine(ast::PredicateExpr::And(vec![
        ast::PredicateExpr::Ge(min.into()),
        ast::PredicateExpr::Le(max.into()),
    ]));
    if let ast::InRangeBounds::Tag(name) = bounds
        && let Some(ast::integer::CanonicalPredicate::HullExcluding { excluded, .. }) = typed
            .tag_types
            .get(&crate::typed::TagId(*name))
            .and_then(|ty| typed.type_registry.integer_validity_for_type(ty))
            .map(|validity| validity.domain().predicate())
    {
        domain = domain.refine(ast::PredicateExpr::And(
            excluded
                .iter()
                .map(|value| ast::PredicateExpr::Ne((*value).into()))
                .collect(),
        ));
    }
    env.integer_domains.insert(version, domain);
}

pub(crate) fn refine_integer_locals_from_condition(
    typed: &TypedFileAst,
    condition: &crate::typed::TypedCondition,
    taken: bool,
    env: &mut FlowState,
) {
    match condition {
        crate::typed::TypedCondition::Proposition { subject } => {
            if let Some(crate::typed::EvidenceProposition::IntegerComparison {
                lhs,
                rhs,
                comparison,
            }) = typed.exprs.proposition_evidence_of(*subject)
            {
                let relation = ast::ProofRelation::from(*comparison);
                refine_comparison_locals(
                    typed,
                    *lhs,
                    if taken {
                        relation
                    } else {
                        relation.complement()
                    },
                    *rhs,
                    env,
                );
            }
        }
        crate::typed::TypedCondition::And(left, right) if taken => {
            refine_integer_locals_from_condition(typed, left, true, env);
            refine_integer_locals_from_condition(typed, right, true, env);
        }
        crate::typed::TypedCondition::Or(left, right) if !taken => {
            refine_integer_locals_from_condition(typed, left, false, env);
            refine_integer_locals_from_condition(typed, right, false, env);
        }
        crate::typed::TypedCondition::Not(inner) => {
            refine_integer_locals_from_condition(typed, inner, !taken, env);
        }
        crate::typed::TypedCondition::Compare {
            left,
            relation,
            right,
        } => {
            refine_comparison_locals(
                typed,
                *left,
                if taken {
                    *relation
                } else {
                    relation.complement()
                },
                *right,
                env,
            );
        }
        crate::typed::TypedCondition::Is { subject, pattern } => {
            refine_union(typed, env, *subject, &pattern.value, taken);
            if taken {
                refine_range(typed, env, *subject, &pattern.value);
            }
            let ast::Pattern::Nominal(selected, _) = &pattern.value else {
                return;
            };
            let selected = if taken {
                *selected
            } else if selected.as_str() == "True" {
                Intern::from_ref("False")
            } else if selected.as_str() == "False" {
                Intern::from_ref("True")
            } else {
                return;
            };
            if let Some(evidence) = typed.exprs.result_evidence_of(*subject)
                && let Some(alternative) = evidence
                    .alternatives
                    .iter()
                    .find(|alternative| alternative.label == selected)
            {
                let crate::typed::EvidenceProposition::IntegerComparison {
                    lhs,
                    rhs,
                    comparison,
                } = alternative.proposition;
                refine_comparison_locals(typed, lhs, comparison.into(), rhs, env);
            }

            let Some(TypedExprKind::FnCall {
                target,
                args: Some(args),
                ..
            }) = typed.exprs.kind_of(*subject)
            else {
                return;
            };
            let Some(bind) = typed.defs.get(target) else {
                return;
            };
            let Ty::ResultFamily { alternatives, .. } = &bind.return_type else {
                return;
            };
            let Some(proposition) = alternatives
                .iter()
                .find(|alternative| alternative.label == selected)
                .and_then(|alternative| alternative.proposition.as_ref())
            else {
                return;
            };
            for ((parameter, _), argument) in bind.params.iter().zip(args) {
                let Some((local, version)) = local_version_for_expr(typed, *argument) else {
                    continue;
                };
                if env.place_version(&local) != Some(version) {
                    continue;
                }
                let predicate = predicate_for_proposition_subject(proposition, *parameter);
                let domain =
                    ast::integer::IntegerDomain::from_named_refinement(&predicate, *parameter);
                if domain.storage_hull().is_some() {
                    env.integer_domains.insert(version, domain.clone());
                    if env.types.get(&local).is_some_and(|ty| {
                        ty.named_instance_stripping_reference_wrappers().is_none()
                    }) {
                        env.types.insert(
                            local,
                            Ty::AnonymousInteger {
                                validity: ast::integer::IntegerValidity::new(domain),
                            },
                        );
                    }
                }
            }
        }
        crate::typed::TypedCondition::And(_, _) | crate::typed::TypedCondition::Or(_, _) => {}
    }
}

fn refine_comparison_locals(
    typed: &TypedFileAst,
    lhs: ExprId,
    comparison: ast::ProofRelation,
    rhs: ExprId,
    env: &mut FlowState,
) {
    for (mut value, bound, reversed) in [(lhs, rhs, false), (rhs, lhs, true)] {
        while let Some(
            TypedExprKind::Ref(inner)
            | TypedExprKind::Deref(inner)
            | TypedExprKind::TakePtr(inner)
            | TypedExprKind::Rematerialize { source: inner, .. },
        ) = typed.exprs.kind_of(value)
        {
            value = *inner;
        }
        let Some((local, version)) = local_version_for_expr(typed, value) else {
            continue;
        };
        let Some(bound) = known_integer_bound(typed, bound) else {
            continue;
        };
        if env.place_version(&local) != Some(version) {
            continue;
        }
        let Some(base) = env.integer_domains.get(&version).cloned().or_else(|| {
            typed.exprs.ty_of(value).and_then(|ty| {
                let value_ty = reference_pointee_for_type(ty, Some(&typed.type_registry))
                    .unwrap_or_else(|| ty.clone());
                typed
                    .type_registry
                    .integer_validity_for_type(&value_ty)
                    .map(|validity| validity.domain().clone())
            })
        }) else {
            continue;
        };
        let bound = bound.into();
        let predicate = match (comparison, reversed) {
            (ast::ProofRelation::Less, false) | (ast::ProofRelation::Greater, true) => {
                ast::PredicateExpr::Lt(bound)
            }
            (ast::ProofRelation::LessOrEqual, false)
            | (ast::ProofRelation::GreaterOrEqual, true) => ast::PredicateExpr::Le(bound),
            (ast::ProofRelation::Greater, false) | (ast::ProofRelation::Less, true) => {
                ast::PredicateExpr::Gt(bound)
            }
            (ast::ProofRelation::GreaterOrEqual, false)
            | (ast::ProofRelation::LessOrEqual, true) => ast::PredicateExpr::Ge(bound),
            (ast::ProofRelation::Equal, _) => ast::PredicateExpr::Eq(bound),
            (ast::ProofRelation::NotEqual, _) => ast::PredicateExpr::Ne(bound),
        };
        env.integer_domains.insert(version, base.refine(predicate));
    }
}

fn known_integer_bound(typed: &TypedFileAst, expression: ExprId) -> Option<i256::I256> {
    if let Some(ConstValue::Int(value)) = typed
        .exprs
        .const_value_of(expression)
        .and_then(Option::as_ref)
    {
        return Some(*value);
    }
    let knowledge = typed
        .exprs
        .integer_knowledge_of(expression)
        .and_then(Option::as_ref)
        .and_then(super::place::knowledge_domain);
    let singleton = |domain: &ast::integer::IntegerDomain| {
        let hull = domain.storage_hull()?;
        (hull.min() == hull.max()).then_some(hull.min())
    };
    knowledge.as_ref().and_then(singleton).or_else(|| {
        singleton(
            typed
                .type_registry
                .integer_validity_for_type(typed.exprs.ty_of(expression)?)?
                .domain(),
        )
    })
}

fn local_version_for_expr(
    typed: &TypedFileAst,
    expr: ExprId,
) -> Option<(Intern<String>, PlaceVersionId)> {
    match typed.exprs.kind_of(expr)? {
        TypedExprKind::FnCall { target, args, .. } if args.is_none() => {
            Some((target.0, typed.exprs.place_version_of(expr).flatten()?))
        }
        TypedExprKind::Ref(inner)
        | TypedExprKind::Deref(inner)
        | TypedExprKind::TakePtr(inner)
        | TypedExprKind::Rematerialize { source: inner, .. } => {
            local_version_for_expr(typed, *inner)
        }
        _ => None,
    }
}

fn predicate_for_proposition_subject(
    proposition: &ast::ProofProposition,
    subject: Intern<String>,
) -> ast::PredicateExpr {
    use ast::{ProofProposition, ProofRelation};
    match proposition {
        ProofProposition::And(left, right) => ast::PredicateExpr::And(vec![
            predicate_for_proposition_subject(left, subject),
            predicate_for_proposition_subject(right, subject),
        ]),
        ProofProposition::Compare {
            left: ast::ProofTerm::Name(name),
            relation,
            right,
        } if *name == subject => {
            let Some(right) = proof_term_as_normal(right) else {
                return ast::PredicateExpr::Proposition(Box::new(proposition.clone()));
            };
            match relation {
                ProofRelation::Equal => ast::PredicateExpr::Eq(right),
                ProofRelation::NotEqual => ast::PredicateExpr::Ne(right),
                ProofRelation::Less => ast::PredicateExpr::Lt(right),
                ProofRelation::LessOrEqual => ast::PredicateExpr::Le(right),
                ProofRelation::Greater => ast::PredicateExpr::Gt(right),
                ProofRelation::GreaterOrEqual => ast::PredicateExpr::Ge(right),
            }
        }
        _ => ast::PredicateExpr::Proposition(Box::new(proposition.clone())),
    }
}

fn proof_term_as_normal(term: &ast::ProofTerm) -> Option<ast::NormalExpr> {
    match term {
        ast::ProofTerm::Value(value) => Some(ast::NormalExpr::Value(ast::ConstValue::Int(*value))),
        ast::ProofTerm::Name(name) => Some(ast::NormalExpr::Var(*name)),
        ast::ProofTerm::Add(left, right) => Some(ast::NormalExpr::Add(
            Box::new(proof_term_as_normal(left)?),
            Box::new(proof_term_as_normal(right)?),
        )),
        ast::ProofTerm::Sub(left, right) => Some(ast::NormalExpr::Sub(
            Box::new(proof_term_as_normal(left)?),
            Box::new(proof_term_as_normal(right)?),
        )),
        ast::ProofTerm::Mul(left, right) => Some(ast::NormalExpr::Mul(
            Box::new(proof_term_as_normal(left)?),
            Box::new(proof_term_as_normal(right)?),
        )),
        ast::ProofTerm::TargetQuery { kind, operand } => Some(ast::NormalExpr::TargetQuery {
            kind: *kind,
            operand: operand.clone(),
        }),
        ast::ProofTerm::Remainder(_, _) | ast::ProofTerm::PowerOfTwo(_) => None,
    }
}

/// Extend `env` with bindings extracted from a pattern, given the
/// subject type.
fn with_pattern_locals(
    typed: &mut TypedFileAst,
    env: &FlowState,
    pattern: &Pattern,
    subject_ty: Option<&Ty>,
    subject_targets: Option<&ReferenceTargetSet>,
    scope: &ExprLowerScope<'_>,
) -> (FlowState, Vec<crate::typed::PlaceId>) {
    let reference = subject_ty
        .and_then(|ty| crate::ty::reference_semantics_for_type(ty, Some(&typed.type_registry)));
    let mut owned_bindings = Vec::new();
    let mut arm_env = env.clone();
    let subject_ty = subject_ty.map(|ty| {
        reference_pointee_for_type(ty, Some(&typed.type_registry)).unwrap_or_else(|| ty.clone())
    });
    let subject_definition = subject_ty
        .as_ref()
        .map(|ty| typed.type_registry.resolved_definition_for_type(ty));
    let mut bindings = pattern.pattern_binding_types(
        if matches!(pattern, Pattern::ListCons { .. }) {
            subject_ty.as_ref()
        } else {
            subject_definition.as_ref()
        },
        scope.variant_map,
        scope.tag_types,
        &|ty| typed.type_registry.sequence_element_type(ty),
    );
    // If the scrutinee has resolved params (e.g. `Vec(Int, 3)`), substitute
    // the opaque type/const params in the pattern bindings with concrete values.
    if let Some(Ty::Union {
        resolved_params: Some(ref params),
        ..
    }) = subject_definition
    {
        let subst = DepSubst::from_ty_args(params);
        for ty in bindings.values_mut() {
            *ty = subst.apply_to_ty(ty);
        }
    }
    if let Some(reference) = &reference {
        for ty in bindings.values_mut() {
            *ty = Ty::Ref {
                inner: Box::new(ty.clone()),
                mutable: reference.permission == crate::type_registry::ReferencePermission::Mutate,
            };
        }
    }
    let binding_names = bindings.keys().copied().collect::<Vec<_>>();
    arm_env.types.extend(bindings);
    add_pattern_target_groups(
        typed,
        &mut arm_env,
        pattern,
        subject_ty.as_ref(),
        subject_targets,
    );
    if reference.is_some()
        && let Some(targets) = subject_targets
    {
        for name in binding_names {
            arm_env
                .target_groups
                .entry(name)
                .or_insert_with(|| targets.clone());
        }
    }
    if reference.is_none() {
        for (slot, name) in pattern.binding_names().into_iter().enumerate() {
            let Some(ty) = arm_env.types.get(&name).cloned() else {
                continue;
            };
            if ty.is_structurally_discardable() {
                continue;
            }
            let place = typed.add_place(crate::typed::TypedPlace {
                binder: ast::BinderId::new(
                    typed.file_id.0,
                    ast::BinderOwner::Pattern {
                        expression: typed.exprs.kind.len() as u32,
                        slot: slot as u32,
                    },
                ),
                name,
                parent: None,
                projection: None,
                mutable: true,
                explicit_contract: true,
                ty: ty.clone(),
            });
            owned_bindings.push(place);
            arm_env.places.insert(name, place);
            let version = typed.add_place_version(
                place,
                crate::typed::PlaceVersionOrigin::Parameter,
                ty,
                None,
            );
            arm_env.place_versions.insert(name, version);
        }
    }
    (arm_env, owned_bindings)
}

pub(super) fn with_condition_locals(
    typed: &mut TypedFileAst,
    env: &FlowState,
    condition: &crate::typed::TypedCondition,
    scope: &ExprLowerScope<'_>,
) -> (FlowState, Vec<crate::typed::PlaceId>) {
    match condition {
        crate::typed::TypedCondition::Is { subject, pattern } => {
            let ty = typed.exprs.ty_of(*subject).cloned();
            let targets = typed.exprs.target_group_of(*subject).cloned().flatten();
            with_pattern_locals(
                typed,
                env,
                &pattern.value,
                ty.as_ref(),
                targets.as_ref(),
                scope,
            )
        }
        crate::typed::TypedCondition::And(left, right) => {
            let (env, mut places) = with_condition_locals(typed, env, left, scope);
            let (env, right_places) = with_condition_locals(typed, &env, right, scope);
            places.extend(right_places);
            (env, places)
        }
        crate::typed::TypedCondition::Or(left, right) => {
            let (left_env, left_places) = with_condition_locals(typed, env, left, scope);
            let (_, right_places) = with_condition_locals(typed, env, right, scope);
            let bindings = |places: &[crate::typed::PlaceId]| {
                places
                    .iter()
                    .map(|place| {
                        let place = &typed.places[place.0 as usize];
                        (place.name, place.ty.clone())
                    })
                    .filter(|(name, _)| !name.as_str().contains('$'))
                    .collect::<std::collections::BTreeMap<_, _>>()
            };
            if bindings(&left_places) == bindings(&right_places) {
                (left_env, left_places)
            } else {
                let mut subjects = Vec::new();
                left.visit_subjects(&mut |subject| subjects.push(subject));
                if let Some(subject) = subjects.first() {
                    // type-incompatible-alternative-bindings
                    let diagnostic = diagnostic::Diagnostic::new(
                        PatternCode::AlternativeBindingsIncompatible,
                        "pattern alternatives must bind the same names with compatible types",
                    )
                    .at_span_id(typed.exprs.span[subject.index()], &typed.span_table);
                    typed.exprs.facts.semantic_facts.flaws[subject.index()].push(diagnostic);
                }
                (env.clone(), Vec::new())
            }
        }
        crate::typed::TypedCondition::Proposition { .. }
        | crate::typed::TypedCondition::Compare { .. }
        | crate::typed::TypedCondition::Not(_) => (env.clone(), Vec::new()),
    }
}

fn add_pattern_target_groups(
    typed: &TypedFileAst,
    env: &mut FlowState,
    pattern: &Pattern,
    subject_ty: Option<&Ty>,
    subject_targets: Option<&ReferenceTargetSet>,
) {
    let Pattern::Generic {
        name, param_spans, ..
    } = pattern
    else {
        return;
    };
    let Some(subject_ty) = subject_ty else {
        return;
    };
    let subject_ty = reference_pointee_for_type(subject_ty, Some(&typed.type_registry))
        .unwrap_or_else(|| subject_ty.clone());
    let subject_definition = typed
        .type_registry
        .resolved_definition_for_type(&subject_ty);
    let Some(variants) = (match &subject_definition {
        Ty::Union { variants, .. } => Some(variants),
        _ => None,
    }) else {
        return;
    };
    let Some(variant) = variants.iter().find(|variant| variant.name == *name) else {
        return;
    };
    let Some(subject_targets) = subject_targets else {
        return;
    };
    let variant_targets = subject_targets.projected(|target| ReferenceTargetGroup::Variant {
        base: Box::new(target.clone()),
        name: *name,
    });
    for (index, (binding, _)) in param_spans.iter().enumerate() {
        if binding.as_str() == "_" || index >= variant.fields.len() {
            continue;
        }
        env.target_groups.insert(
            *binding,
            variant_targets.projected(|target| ReferenceTargetGroup::Field {
                base: Box::new(target.clone()),
                index,
            }),
        );
    }
}

/// Lower the body of a single when arm, optionally extending the local
/// environment with pattern bindings.
pub(crate) fn lower_when_arm_body(
    typed: &mut crate::typed::TypedFileAst,
    body: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &FlowState,
    pattern: Option<&Pattern>,
    subject_ty: Option<&Ty>,
    subject_targets: Option<&ReferenceTargetSet>,
) -> (ExprId, FlowState, Vec<crate::typed::PlaceId>) {
    let (mut arm_env, owned_bindings) = pattern
        .map(|pat| with_pattern_locals(typed, env, pat, subject_ty, subject_targets, scope))
        .unwrap_or_else(|| (env.clone(), Vec::new()));
    let temporary = scope.child();
    let mut body_scope = temporary.with_expected(scope.expected_ty);
    body_scope.value_required = scope.value_required;
    let body = lower_typed_expr(typed, body, &body_scope, &mut arm_env);
    (body, arm_env, owned_bindings)
}

/// Lower all when arms unconditionally (runtime path).
pub(crate) fn lower_all_when_arms(
    typed: &mut crate::typed::TypedFileAst,
    arms: &[WhenArm],
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
    subject_ty: Option<&Ty>,
    subject_targets: Option<&ReferenceTargetSet>,
    subject: Option<ExprId>,
) -> Vec<crate::typed::TypedWhenArm> {
    if subject_ty.is_some_and(|ty| matches!(ty.without_reference_wrappers(), Ty::Proposition(_))) {
        let subject = subject.expect("pattern matching has a subject");
        typed.declaration_flaws.push((
            typed.exprs.span[subject.index()],
            // type-proposition-pattern
            diagnostic::Diagnostic::new(
                PatternCode::Proposition,
                "propositions are tested directly; remove the 'is' pattern",
            )
            .at_span_id(typed.exprs.span[subject.index()], &typed.span_table),
        ));
    }
    let incoming = env.clone();
    let mut fallthrough = env.clone();
    let mut branch_envs = Vec::with_capacity(arms.len());
    let lowered: Vec<crate::typed::TypedWhenArm> = arms
        .iter()
        .map(|arm| match arm {
            WhenArm::Cond {
                condition,
                body,
                arm_span,
            } => {
                let coverage = super::when_coverage::condition_coverage(
                    condition,
                    scope.when_subject.map(|(span, _)| span),
                );
                let coverage = coverage
                    .into_iter()
                    .map(|pattern| {
                        resolve_range_pattern(
                            typed,
                            &ast::Spanned::new(pattern, ast::SpanId::INVALID),
                            scope,
                        )
                        .value
                    })
                    .collect();
                let condition = lower_condition(typed, condition, scope, &mut fallthrough);
                let (mut guarded, owned_bindings) =
                    with_condition_locals(typed, &fallthrough, &condition, scope);
                refine_integer_locals_from_condition(typed, &condition, true, &mut guarded);
                refine_integer_locals_from_condition(typed, &condition, false, &mut fallthrough);
                let (body_id, arm_env, _owned_bindings) = lower_when_arm_body(
                    typed,
                    body,
                    scope,
                    &guarded,
                    None,
                    subject_ty,
                    subject_targets,
                );
                if !crate::analysis::expression_returns(typed.resolved_program(), body_id) {
                    branch_envs.push(arm_env);
                }
                crate::typed::TypedWhenArm::Cond {
                    condition,
                    coverage,
                    body: body_id,
                    owned_bindings,
                    arm_span: *arm_span,
                }
            }
            WhenArm::Is {
                pattern,
                body,
                arm_span,
            } => {
                let mut guarded = fallthrough.clone();
                if let Some(subject) = subject {
                    let condition = crate::typed::TypedCondition::Is {
                        subject,
                        pattern: Box::new(resolve_range_pattern(typed, pattern, scope)),
                    };
                    refine_integer_locals_from_condition(typed, &condition, true, &mut guarded);
                    refine_integer_locals_from_condition(
                        typed,
                        &condition,
                        false,
                        &mut fallthrough,
                    );
                }
                let (body_id, arm_env, owned_bindings) = lower_when_arm_body(
                    typed,
                    body,
                    scope,
                    &guarded,
                    Some(&pattern.value),
                    subject_ty,
                    subject_targets,
                );
                if !crate::analysis::expression_returns(typed.resolved_program(), body_id) {
                    branch_envs.push(arm_env);
                }
                crate::typed::TypedWhenArm::Is {
                    owned_bindings,
                    pattern: Box::new(resolve_range_pattern(typed, pattern, scope)),
                    body: body_id,
                    arm_span: *arm_span,
                }
            }
            WhenArm::Else(body, arm_span) => {
                let (body_id, arm_env, _owned_bindings) = lower_when_arm_body(
                    typed,
                    body,
                    scope,
                    &fallthrough,
                    None,
                    subject_ty,
                    subject_targets,
                );
                if !crate::analysis::expression_returns(typed.resolved_program(), body_id) {
                    branch_envs.push(arm_env);
                }
                crate::typed::TypedWhenArm::Else(body_id, *arm_span)
            }
        })
        .collect();
    let subject_definition =
        subject_ty.map(|ty| typed.type_registry.resolved_definition_for_type(ty));
    let has_else = arms.iter().any(|arm| matches!(arm, WhenArm::Else(_, _)));
    if !has_else
        && !crate::analysis::when_is_exhaustive(
            subject_definition.as_ref(),
            &lowered,
            &typed.type_registry,
        )
    {
        branch_envs.push(fallthrough);
    }
    if let Some(first) = branch_envs.first() {
        env.integer_domains = first.integer_domains.clone();
        env.union_members = first.union_members.clone();
    }
    env.const_values.retain(|name, value| {
        branch_envs
            .iter()
            .all(|branch| branch.const_values.get(name) == Some(value))
    });
    join_place_versions(typed, env, &incoming, &branch_envs, false);
    lowered
}
