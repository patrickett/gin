use std::collections::{HashMap, HashSet};

use ast::ParamConvention;

use crate::ty::Ty;
use crate::typed::{
    AppliedEffectTarget, BindBody, DefId, EffectTarget, FunctionEffects, TypedCallableSignature,
    TypedExprKind, TypedFileAst,
};

use super::flow::children_of_kind;
use super::lower_exprs::target_index_for_expr;

pub fn stage_infer_effects(
    typed: &mut TypedFileAst,
    cross_file_signatures: &HashMap<DefId, TypedCallableSignature>,
) {
    let mut def_ids: Vec<_> = typed.defs.keys().copied().collect();
    def_ids.sort_by_key(|def_id| def_id.0.as_str().to_string());

    loop {
        let mut signatures = cross_file_signatures.clone();
        signatures.extend(
            typed
                .defs
                .iter()
                .map(|(def_id, bind)| (*def_id, TypedCallableSignature::from(bind))),
        );
        let mut changed = false;

        for def_id in &def_ids {
            let Some(bind) = typed.defs.get(def_id) else {
                continue;
            };
            let effects = infer_bind_effects(typed, bind, &signatures);
            if effects != bind.effects {
                typed.defs.get_mut(def_id).unwrap().effects = effects;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }
}

pub fn stage_infer_package_effects(
    typed_files: &mut [TypedFileAst],
    external_signatures: &HashMap<DefId, TypedCallableSignature>,
) {
    loop {
        let mut signatures = external_signatures.clone();
        for typed in typed_files.iter() {
            signatures.extend(
                typed
                    .defs
                    .iter()
                    .map(|(def_id, bind)| (*def_id, TypedCallableSignature::from(bind))),
            );
        }

        let updates: Vec<_> = typed_files
            .iter()
            .enumerate()
            .flat_map(|(file_index, typed)| {
                let signatures = &signatures;
                typed.defs.iter().filter_map(move |(def_id, bind)| {
                    let effects = infer_bind_effects(typed, bind, signatures);
                    (effects != bind.effects).then_some((file_index, *def_id, effects))
                })
            })
            .collect();

        if updates.is_empty() {
            break;
        }
        for (file_index, def_id, effects) in updates {
            typed_files[file_index]
                .defs
                .get_mut(&def_id)
                .unwrap()
                .effects = effects;
        }
    }
}

fn infer_bind_effects(
    typed: &TypedFileAst,
    bind: &crate::typed::TypedBind,
    signatures: &HashMap<DefId, TypedCallableSignature>,
) -> FunctionEffects {
    let roots = match &bind.body {
        BindBody::Expr(expr) => vec![*expr],
        BindBody::Body { exprs, ret } => {
            let mut roots = exprs.clone();
            roots.extend(ret);
            roots
        }
        BindBody::Extern => Vec::new(),
    };
    let mut effects = FunctionEffects::default();
    let mut visited = HashSet::new();
    for root in roots {
        infer_expr_effects(typed, root, signatures, &mut effects, &mut visited);
    }
    effects
}

fn infer_expr_effects(
    typed: &TypedFileAst,
    expr_id: crate::typed::ExprId,
    signatures: &HashMap<DefId, TypedCallableSignature>,
    effects: &mut FunctionEffects,
    visited: &mut HashSet<crate::typed::ExprId>,
) {
    if !visited.insert(expr_id) {
        return;
    }
    let idx = expr_id.as_usize();
    let Some(kind) = typed.exprs.kind.get(idx) else {
        return;
    };

    add_exact_effect(typed, expr_id, &mut effects.reads);

    match kind {
        TypedExprKind::RecordSet { base, field, .. } => {
            if let Some(index) = record_field_index(typed, *base, *field) {
                add_projected_effect(typed, *base, &mut effects.writes, |base| {
                    EffectTarget::Field {
                        base: Box::new(base),
                        index,
                    }
                });
                add_projected_effect(typed, *base, &mut effects.invalidates, |base| {
                    EffectTarget::Descendants(Box::new(EffectTarget::Field {
                        base: Box::new(base),
                        index,
                    }))
                });
            }
        }
        TypedExprKind::TupleSet { base, index, .. } => {
            add_projected_effect(typed, *base, &mut effects.writes, |base| {
                EffectTarget::Field {
                    base: Box::new(base),
                    index: *index,
                }
            });
            add_projected_effect(typed, *base, &mut effects.invalidates, |base| {
                EffectTarget::Descendants(Box::new(EffectTarget::Field {
                    base: Box::new(base),
                    index: *index,
                }))
            });
        }
        TypedExprKind::BufSet { buf, index, .. } => {
            let index = target_index_for_expr(typed, *index);
            add_projected_effect(typed, *buf, &mut effects.writes, |base| {
                EffectTarget::ItemRegion {
                    base: Box::new(base),
                    index: index.clone(),
                }
            });
            add_projected_effect(typed, *buf, &mut effects.invalidates, |base| {
                EffectTarget::Descendants(Box::new(EffectTarget::ItemRegion {
                    base: Box::new(base),
                    index: index.clone(),
                }))
            });
        }
        TypedExprKind::Eat(inner) | TypedExprKind::ConsumeArg(inner) => {
            add_exact_effect(typed, *inner, &mut effects.consumes);
            add_descendant_effect(typed, *inner, &mut effects.invalidates);
        }
        TypedExprKind::FnCall {
            target,
            args: Some(args),
            ..
        } => {
            let signature = signatures.get(target);
            if let Some(signature) = signature {
                apply_call_effects(typed, args, signature, effects);
            }
        }
        _ => {}
    }

    for child in children_of_kind(kind) {
        infer_expr_effects(typed, child, signatures, effects, visited);
    }
}

fn apply_call_effects(
    typed: &TypedFileAst,
    args: &[crate::typed::ExprId],
    signature: &TypedCallableSignature,
    effects: &mut FunctionEffects,
) {
    for (index, convention) in signature.param_conventions.iter().enumerate() {
        let Some(arg) = args.get(index) else {
            continue;
        };
        match convention {
            ParamConvention::Observe => add_exact_effect(typed, *arg, &mut effects.reads),
            ParamConvention::Mutate => add_exact_effect(typed, *arg, &mut effects.writes),
            ParamConvention::Consume => {
                add_exact_effect(typed, *arg, &mut effects.consumes);
                add_descendant_effect(typed, *arg, &mut effects.invalidates);
            }
            ParamConvention::Own => {}
        }
    }

    substitute_effects(
        typed,
        args,
        signature,
        &signature.effects.reads,
        &mut effects.reads,
    );
    substitute_effects(
        typed,
        args,
        signature,
        &signature.effects.writes,
        &mut effects.writes,
    );
    substitute_effects(
        typed,
        args,
        signature,
        &signature.effects.invalidates,
        &mut effects.invalidates,
    );
    substitute_effects(
        typed,
        args,
        signature,
        &signature.effects.consumes,
        &mut effects.consumes,
    );
}

fn substitute_effects(
    typed: &TypedFileAst,
    args: &[crate::typed::ExprId],
    signature: &TypedCallableSignature,
    callee_targets: &HashSet<EffectTarget>,
    caller_targets: &mut HashSet<EffectTarget>,
) {
    for applied in applied_effect_targets(typed, args, signature, callee_targets) {
        if let Some(mut target) = EffectTarget::from_reference(&applied.target) {
            if applied.descendants {
                target = EffectTarget::Descendants(Box::new(target));
            }
            caller_targets.insert(target);
        }
    }
}

pub(crate) fn applied_effect_targets(
    typed: &TypedFileAst,
    args: &[crate::typed::ExprId],
    signature: &TypedCallableSignature,
    effects: &HashSet<EffectTarget>,
) -> Vec<AppliedEffectTarget> {
    let mut applied = Vec::new();
    for effect in effects {
        for (index, group) in signature.param_groups.iter().enumerate() {
            let Some(group) = group else {
                continue;
            };
            let Some(arg) = args.get(index) else {
                continue;
            };
            let Some(actual) = typed
                .exprs
                .target_group
                .get(arg.as_usize())
                .and_then(|target| target.as_ref())
            else {
                continue;
            };
            for actual in actual.iter() {
                if let Some(target) = effect.substitute(*group, actual)
                    && !applied.contains(&target)
                {
                    applied.push(target);
                }
            }
        }
    }
    applied
}

fn add_exact_effect(
    typed: &TypedFileAst,
    expr_id: crate::typed::ExprId,
    effects: &mut HashSet<EffectTarget>,
) {
    let Some(target) = typed
        .exprs
        .target_group
        .get(expr_id.as_usize())
        .and_then(|target| target.as_ref())
    else {
        return;
    };
    for target in target.iter() {
        if let Some(target) = EffectTarget::from_reference(target) {
            effects.insert(target);
        }
    }
}

fn add_descendant_effect(
    typed: &TypedFileAst,
    expr_id: crate::typed::ExprId,
    effects: &mut HashSet<EffectTarget>,
) {
    let Some(target) = typed
        .exprs
        .target_group
        .get(expr_id.as_usize())
        .and_then(|target| target.as_ref())
    else {
        return;
    };
    for target in target.iter() {
        if let Some(target) = EffectTarget::from_reference(target) {
            effects.insert(EffectTarget::Descendants(Box::new(target)));
        }
    }
}

fn add_projected_effect(
    typed: &TypedFileAst,
    expr_id: crate::typed::ExprId,
    effects: &mut HashSet<EffectTarget>,
    mut project: impl FnMut(EffectTarget) -> EffectTarget,
) {
    let Some(target) = typed
        .exprs
        .target_group
        .get(expr_id.as_usize())
        .and_then(|target| target.as_ref())
    else {
        return;
    };
    for target in target.iter() {
        if let Some(target) = EffectTarget::from_reference(target) {
            effects.insert(project(target));
        }
    }
}

fn record_field_index(
    typed: &TypedFileAst,
    base: crate::typed::ExprId,
    field: internment::Intern<String>,
) -> Option<usize> {
    let mut ty = typed.exprs.ty.get(base.as_usize())?;
    if let Ty::Ref { inner, .. } = ty {
        ty = inner;
    }
    let Ty::Record { fields, .. } = ty else {
        return None;
    };
    fields.iter().position(|(name, _)| *name == field)
}
