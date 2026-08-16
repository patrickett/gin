//! When-arm lowering helpers.
//!
//! Provides functions for lowering the arms of `when` expressions,
//! including pattern-local variable binding and compile-time constant
//! arm selection.

use ast::prelude::*;

use crate::subst::DepSubst;
use crate::ty::{Ty, reference_pointee_for_type};
use crate::typed::{ExprId, ReferenceTargetGroup, ReferenceTargetSet, TypedFileAst};

use super::lower_exprs::lower_typed_expr;
use super::lower_exprs::{ExprLowerScope, LocalEnv, join_place_versions};

pub(crate) fn lower_condition(
    typed: &mut TypedFileAst,
    condition: &ast::Condition,
    scope: &ExprLowerScope<'_>,
    env: &mut LocalEnv,
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

/// Extend `env` with bindings extracted from a pattern, given the
/// subject type.
fn with_pattern_locals(
    typed: &TypedFileAst,
    env: &LocalEnv,
    pattern: &Pattern,
    subject_ty: Option<&Ty>,
    subject_targets: Option<&ReferenceTargetSet>,
    scope: &ExprLowerScope<'_>,
) -> LocalEnv {
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
    arm_env.types.extend(pattern.pattern_binding_types(
        subject_ty.as_ref(),
        scope.variant_map,
        &pattern_tag_types,
    ));
    // If the scrutinee has resolved params (e.g. `Vec(Int, 3)`), substitute
    // the opaque type/const params in the pattern bindings with concrete values.
    let subject_definition = subject_ty
        .as_ref()
        .map(|ty| typed.type_registry.resolved_definition_for_type(ty));
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
    env: &mut LocalEnv,
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
    env: &LocalEnv,
    pattern: Option<&Pattern>,
    subject_ty: Option<&Ty>,
    subject_targets: Option<&ReferenceTargetSet>,
) -> (ExprId, LocalEnv) {
    let mut arm_env = pattern
        .map(|pat| with_pattern_locals(typed, env, pat, subject_ty, subject_targets, scope))
        .unwrap_or_else(|| env.clone());
    let body = lower_typed_expr(typed, body, &scope.temporary_child(), &mut arm_env);
    (body, arm_env)
}

/// Lower all when arms unconditionally (runtime path).
pub(crate) fn lower_all_when_arms(
    typed: &mut crate::typed::TypedFileAst,
    arms: &[WhenArm],
    scope: &ExprLowerScope<'_>,
    env: &mut LocalEnv,
    subject_ty: Option<&Ty>,
    subject_targets: Option<&ReferenceTargetSet>,
) -> Vec<crate::typed::TypedWhenArm> {
    let incoming = env.clone();
    let mut branch_envs = Vec::with_capacity(arms.len());
    let lowered = arms
        .iter()
        .map(|arm| match arm {
            WhenArm::Cond {
                condition,
                body,
                arm_span,
            } => {
                let condition = lower_condition(typed, condition, scope, env);
                let (body_id, arm_env) =
                    lower_when_arm_body(typed, body, scope, env, None, subject_ty, subject_targets);
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
                let (body_id, arm_env) = lower_when_arm_body(
                    typed,
                    body,
                    scope,
                    env,
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
                let (body_id, arm_env) =
                    lower_when_arm_body(typed, body, scope, env, None, subject_ty, subject_targets);
                branch_envs.push(arm_env);
                crate::typed::TypedWhenArm::Else(body_id, *arm_span)
            }
        })
        .collect();
    let include_incoming = !arms.iter().any(|arm| matches!(arm, WhenArm::Else(_, _)));
    join_place_versions(typed, env, &incoming, &branch_envs, include_incoming);
    lowered
}
