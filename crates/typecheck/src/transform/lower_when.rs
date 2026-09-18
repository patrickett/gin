//! When-arm lowering helpers.
//!
//! Provides functions for lowering the arms of `when` expressions,
//! including pattern-local variable binding and compile-time constant
//! arm selection.

use ast::prelude::*;

use crate::subst::DepSubst;
use crate::ty::{Ty, reference_pointee_for_type};
use crate::typed::{ExprId, ReferenceTargetGroup, ReferenceTargetSet, TypedExprKind, TypedFileAst};

use super::lower_exprs::lower_typed_expr;
use super::lower_exprs::{ExprLowerScope, FlowState, join_place_versions};

pub(crate) fn lower_condition(
    typed: &mut TypedFileAst,
    condition: &ast::Condition,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> crate::typed::TypedCondition {
    match condition {
        ast::Condition::Is { subject, pattern } => crate::typed::TypedCondition::Is {
            subject: lower_typed_expr(typed, subject, &scope.child(), env),
            pattern: pattern.clone(),
        },
        ast::Condition::Not(inner) => {
            crate::typed::TypedCondition::Not(Box::new(lower_condition(typed, inner, scope, env)))
        }
        ast::Condition::And(left, right) => crate::typed::TypedCondition::And(
            Box::new(lower_condition(typed, left, scope, env)),
            Box::new(lower_condition(typed, right, scope, env)),
        ),
        ast::Condition::Or(left, right) => crate::typed::TypedCondition::Or(
            Box::new(lower_condition(typed, left, scope, env)),
            Box::new(lower_condition(typed, right, scope, env)),
        ),
    }
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
    let domain = ast::integer::IntegerDomain::bounded(min, max)
        .unwrap_or_else(ast::integer::IntegerDomain::empty);
    env.integer_domains.insert(version, domain);
}

pub(crate) fn refine_integer_locals_from_condition(
    typed: &TypedFileAst,
    condition: &crate::typed::TypedCondition,
    taken: bool,
    env: &mut FlowState,
) {
    match condition {
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
                let Some(local) = local_name_for_expr(typed, *argument) else {
                    continue;
                };
                let predicate = predicate_for_proposition_subject(proposition, *parameter);
                let domain =
                    ast::integer::IntegerDomain::from_named_refinement(&predicate, *parameter);
                if domain.storage_hull().is_some() {
                    if let Some(version) = env.place_version(&local) {
                        env.integer_domains.insert(version, domain.clone());
                    }
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

fn local_name_for_expr(typed: &TypedFileAst, expr: ExprId) -> Option<Intern<String>> {
    match typed.exprs.kind_of(expr)? {
        TypedExprKind::FnCall { target, args, .. } if args.is_none() => Some(target.0),
        TypedExprKind::Ref(inner)
        | TypedExprKind::Deref(inner)
        | TypedExprKind::TakePtr(inner)
        | TypedExprKind::Rematerialize { source: inner, .. } => local_name_for_expr(typed, *inner),
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
    typed: &TypedFileAst,
    env: &FlowState,
    pattern: &Pattern,
    subject_ty: Option<&Ty>,
    subject_targets: Option<&ReferenceTargetSet>,
    scope: &ExprLowerScope<'_>,
) -> FlowState {
    let mut arm_env = env.clone();
    let subject_ty = subject_ty.map(|ty| {
        reference_pointee_for_type(ty, Some(&typed.type_registry)).unwrap_or_else(|| ty.clone())
    });
    let mut pattern_tag_types = scope.tag_types.clone();
    let list_name = Intern::from_ref("List");
    if let Some(list_ty) = pattern_tag_types.get(&list_name).cloned() {
        pattern_tag_types.insert(
            list_name,
            typed.type_registry.resolved_definition_for_type(&list_ty),
        );
    }
    let subject_definition = subject_ty
        .as_ref()
        .map(|ty| typed.type_registry.resolved_definition_for_type(ty));
    arm_env.types.extend(pattern.pattern_binding_types(
        subject_definition.as_ref(),
        scope.variant_map,
        &pattern_tag_types,
    ));
    // If the scrutinee has resolved params (e.g. `Vec(Int, 3)`), substitute
    // the opaque type/const params in the pattern bindings with concrete values.
    if let Some(Ty::Union {
        resolved_params: Some(params),
        ..
    }) = subject_definition
    {
        let subst = DepSubst::from_ty_args(&params);
        for ty in arm_env.types.values_mut() {
            *ty = subst.apply_to_ty(ty);
        }
    }
    add_pattern_target_groups(
        typed,
        &mut arm_env,
        pattern,
        subject_ty.as_ref(),
        subject_targets,
    );
    arm_env
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
) -> (ExprId, FlowState) {
    let mut arm_env = pattern
        .map(|pat| with_pattern_locals(typed, env, pat, subject_ty, subject_targets, scope))
        .unwrap_or_else(|| env.clone());
    let body = lower_typed_expr(
        typed,
        body,
        &scope.temporary_child().with_expected(scope.expected_ty),
        &mut arm_env,
    );
    (body, arm_env)
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
    let incoming = env.clone();
    let mut fallthrough = env.clone();
    let mut branch_envs = Vec::with_capacity(arms.len());
    let lowered = arms
        .iter()
        .map(|arm| match arm {
            WhenArm::Cond {
                condition,
                body,
                arm_span,
            } => {
                let condition = lower_condition(typed, condition, scope, &mut fallthrough);
                let mut guarded = fallthrough.clone();
                refine_integer_locals_from_condition(typed, &condition, true, &mut guarded);
                refine_integer_locals_from_condition(typed, &condition, false, &mut fallthrough);
                let (body_id, arm_env) = lower_when_arm_body(
                    typed,
                    body,
                    scope,
                    &guarded,
                    None,
                    subject_ty,
                    subject_targets,
                );
                branch_envs.push(arm_env);
                crate::typed::TypedWhenArm::Cond {
                    condition,
                    body: body_id,
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
                    refine_range(typed, &mut guarded, subject, &pattern.value);
                    refine_union(typed, &mut guarded, subject, &pattern.value, true);
                    refine_union(typed, &mut fallthrough, subject, &pattern.value, false);
                }
                let (body_id, arm_env) = lower_when_arm_body(
                    typed,
                    body,
                    scope,
                    &guarded,
                    Some(&pattern.value),
                    subject_ty,
                    subject_targets,
                );
                branch_envs.push(arm_env);
                crate::typed::TypedWhenArm::Is {
                    pattern: pattern.clone(),
                    body: body_id,
                    arm_span: *arm_span,
                }
            }
            WhenArm::Else(body, arm_span) => {
                let (body_id, arm_env) = lower_when_arm_body(
                    typed,
                    body,
                    scope,
                    &fallthrough,
                    None,
                    subject_ty,
                    subject_targets,
                );
                branch_envs.push(arm_env);
                crate::typed::TypedWhenArm::Else(body_id, *arm_span)
            }
        })
        .collect();
    let include_incoming = !arms.iter().any(|arm| matches!(arm, WhenArm::Else(_, _)));
    join_place_versions(typed, env, &incoming, &branch_envs, include_incoming);
    lowered
}
