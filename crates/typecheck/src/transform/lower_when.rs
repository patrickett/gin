//! When-arm lowering helpers.
//!
//! Provides functions for lowering the arms of `when` expressions,
//! including pattern-local variable binding and compile-time constant
//! arm selection.

use ast::prelude::*;

use crate::subst::DepSubst;
use crate::ty::Ty;
use crate::typed::ExprId;

use super::lower_exprs::lower_typed_expr;
use super::lower_exprs::{ExprLowerScope, LocalEnv};

/// Extend `env` with bindings extracted from a pattern, given the
/// subject type.
fn with_pattern_locals(
    env: &LocalEnv,
    pattern: &TypeExpr,
    subject_ty: Option<&Ty>,
    scope: &ExprLowerScope<'_>,
) -> LocalEnv {
    let mut arm_env = env.clone();
    arm_env.types.extend(pattern.pattern_binding_types(
        subject_ty,
        scope.variant_map,
        scope.tag_types,
    ));
    // If the scrutinee has resolved params (e.g. `Vec(Int, 3)`), substitute
    // the opaque type/const params in the pattern bindings with concrete values.
    if let Some(Ty::Union {
        resolved_params: Some(params),
        ..
    }) = subject_ty
    {
        let subst = DepSubst::from_ty_args(params);
        for ty in arm_env.types.values_mut() {
            *ty = subst.apply_to_ty(ty);
        }
    }
    arm_env
}

/// Lower the body of a single when arm, optionally extending the local
/// environment with pattern bindings.
pub(crate) fn lower_when_arm_body(
    typed: &mut crate::typed::TypedFileAst,
    body: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &LocalEnv,
    pattern: Option<&TypeExpr>,
    subject_ty: Option<&Ty>,
) -> ExprId {
    let mut arm_env = pattern
        .map(|pat| with_pattern_locals(env, pat, subject_ty, scope))
        .unwrap_or_else(|| env.clone());
    lower_typed_expr(typed, body, scope, &mut arm_env)
}

/// Lower all when arms unconditionally (runtime path).
pub(crate) fn lower_all_when_arms(
    typed: &mut crate::typed::TypedFileAst,
    arms: &[WhenArm],
    scope: &ExprLowerScope<'_>,
    env: &mut LocalEnv,
    subject_ty: Option<&Ty>,
) -> Vec<crate::typed::TypedWhenArm> {
    arms.iter()
        .map(|arm| match arm {
            WhenArm::Cond {
                condition,
                body,
                arm_span,
            } => {
                let cond_id = lower_typed_expr(typed, condition, &scope.child(), env);
                let body_id = lower_when_arm_body(typed, body, scope, env, None, subject_ty);
                crate::typed::TypedWhenArm::Cond {
                    condition: cond_id,
                    body: body_id,
                    arm_span: *arm_span,
                }
            }
            WhenArm::Is {
                pattern,
                body,
                arm_span,
            } => {
                let body_id =
                    lower_when_arm_body(typed, body, scope, env, Some(&pattern.value), subject_ty);
                crate::typed::TypedWhenArm::Is {
                    pattern: pattern.clone(),
                    body: body_id,
                    arm_span: *arm_span,
                }
            }
            WhenArm::Else(body, arm_span) => {
                let body_id = lower_when_arm_body(typed, body, scope, env, None, subject_ty);
                crate::typed::TypedWhenArm::Else(body_id, *arm_span)
            }
        })
        .collect()
}
