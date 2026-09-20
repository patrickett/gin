use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use ast::{BinderId, ParamConvention, TyArg};
use internment::Intern;

use crate::ty::{Ty, TyChild, reference_pointee_for_type};
use crate::typed::{
    AppliedEffectTarget, AvailabilityRequirement, BindBody, CallCapability, DefId, DefinitionStage,
    EffectTarget, FunctionEffects, TypedCallableSignature, TypedExprKind, TypedFileAst,
    walk_expr_children,
};

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
            let summary = infer_bind_summary(typed, bind, &signatures);
            if summary.effects != bind.effects
                || summary.param_requirements != bind.param_requirements
                || summary.call_capability != bind.call_capability
                || summary.definition_stage != bind.definition_stage
            {
                let typed_bind = typed.defs.get_mut(def_id).unwrap();
                typed_bind.effects = summary.effects;
                typed_bind.param_requirements = summary.param_requirements;
                typed_bind.call_capability = summary.call_capability;
                typed_bind.definition_stage = summary.definition_stage;
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
                    let summary = infer_bind_summary(typed, bind, signatures);
                    (summary.effects != bind.effects
                        || summary.param_requirements != bind.param_requirements
                        || summary.call_capability != bind.call_capability
                        || summary.definition_stage != bind.definition_stage)
                        .then_some((file_index, *def_id, summary))
                })
            })
            .collect();

        if updates.is_empty() {
            break;
        }
        for (file_index, def_id, summary) in updates {
            let bind = typed_files[file_index].defs.get_mut(&def_id).unwrap();
            bind.effects = summary.effects;
            bind.param_requirements = summary.param_requirements;
            bind.call_capability = summary.call_capability;
            bind.definition_stage = summary.definition_stage;
        }
    }
}

#[derive(Debug, Clone)]
struct CallableSummary {
    effects: FunctionEffects,
    param_requirements: Vec<AvailabilityRequirement>,
    call_capability: CallCapability,
    definition_stage: DefinitionStage,
}

fn infer_bind_summary(
    typed: &TypedFileAst,
    bind: &crate::typed::TypedBind,
    signatures: &HashMap<DefId, TypedCallableSignature>,
) -> CallableSummary {
    let roots = match &bind.body {
        BindBody::Expr(expr) => vec![*expr],
        BindBody::Body { exprs, ret } => {
            let mut roots = exprs.clone();
            roots.extend(ret);
            roots
        }
        BindBody::Unlowered => Vec::new(),
    };
    let mut effects = FunctionEffects::default();
    let mut has_runtime_call = false;
    let mut visited = HashSet::new();
    for root in &roots {
        infer_expr_effects(
            typed,
            *root,
            signatures,
            &mut has_runtime_call,
            &mut effects,
            &mut visited,
        );
    }
    let param_requirements = infer_param_requirements(bind, typed);
    let call_capability = if matches!(bind.body, crate::typed::BindBody::Unlowered)
        || effects.unknown_memory
        || has_runtime_call
    {
        CallCapability::RuntimeOnly
    } else {
        CallCapability::StagePolymorphic
    };

    CallableSummary {
        effects,
        param_requirements,
        call_capability,
        definition_stage: if call_capability == CallCapability::RuntimeOnly {
            DefinitionStage::RuntimeOnly
        } else if roots
            .iter()
            .any(|root| body_has_compile_time_boundary(typed, *root, signatures))
        {
            DefinitionStage::CompileTimeOnly
        } else {
            DefinitionStage::StagePolymorphic
        },
    }
}

fn body_has_compile_time_boundary(
    typed: &TypedFileAst,
    root: crate::typed::ExprId,
    signatures: &HashMap<DefId, TypedCallableSignature>,
) -> bool {
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        match typed.exprs.kind_of(id) {
            Some(TypedExprKind::Asm { operands, .. }) => {
                pending.extend(operands.iter().map(|(_, operand)| *operand));
                continue;
            }
            Some(TypedExprKind::Target | TypedExprKind::TargetField(_)) => return true,
            Some(TypedExprKind::FnCall { target, .. }) => {
                if signatures
                    .get(target)
                    .is_some_and(|s| s.definition_stage == DefinitionStage::CompileTimeOnly)
                {
                    return true;
                }
            }
            _ => {}
        }
        let _ = crate::typed::walk_expr_children_of(typed, id, &mut |child| {
            pending.push(child);
            std::ops::ControlFlow::Continue(())
        });
    }
    false
}

fn infer_param_requirements(
    bind: &crate::typed::TypedBind,
    typed: &TypedFileAst,
) -> Vec<AvailabilityRequirement> {
    let param_count = bind.params.len();
    let mut name_to_index = HashMap::with_capacity(param_count);
    let mut inferred_index = HashMap::with_capacity(param_count);
    for (index, (param_name, _)) in bind.params.iter().enumerate() {
        name_to_index.insert(*param_name, index);
        inferred_index.insert(index as u32, index);
    }

    let mut required = vec![false; param_count];
    let mut dependencies = vec![Vec::new(); param_count];

    let direct_return = param_indices_in_ty(
        &bind.return_type,
        &name_to_index,
        &inferred_index,
        bind.dependent_binder,
    );
    for index in direct_return {
        required[index] = true;
    }

    for (index, (_, param_ty)) in bind.params.iter().enumerate() {
        let deps = param_indices_in_ty(
            param_ty,
            &name_to_index,
            &inferred_index,
            bind.dependent_binder,
        );
        for dep in deps {
            if dep == index {
                continue;
            }
            required[dep] = true;
            if !dependencies[index].contains(&dep) {
                dependencies[index].push(dep);
            }
        }
        if matches!(bind.param_kinds[index], crate::ty::ParamKind::Type) {
            required[index] = true;
        }
    }

    for expr_id in bind_body_exprs(bind, typed) {
        let Some(expr_ty) = typed.exprs.ty.get(expr_id.as_usize()) else {
            continue;
        };
        for index in param_indices_in_ty(
            expr_ty,
            &name_to_index,
            &inferred_index,
            bind.dependent_binder,
        ) {
            required[index] = true;
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for index in 0..param_count {
            if required[index] {
                continue;
            }
            if dependencies[index]
                .iter()
                .any(|dependency| required[*dependency])
            {
                required[index] = true;
                changed = true;
            }
        }
    }

    required
        .into_iter()
        .map(|is_required| {
            if is_required {
                AvailabilityRequirement::CompileTime
            } else {
                AvailabilityRequirement::Any
            }
        })
        .collect()
}

fn has_runtime_effects(effects: &FunctionEffects) -> bool {
    effects.unknown_memory
        || !effects.reads.is_empty()
        || !effects.writes.is_empty()
        || !effects.invalidates.is_empty()
        || !effects.consumes.is_empty()
}

fn bind_body_exprs<'a>(
    bind: &'a crate::typed::TypedBind,
    _typed: &'a TypedFileAst,
) -> Vec<crate::typed::ExprId> {
    match &bind.body {
        BindBody::Expr(expr) => vec![*expr],
        BindBody::Body { exprs, ret } => {
            let mut expr_ids = exprs.clone();
            expr_ids.extend(ret);
            expr_ids
        }
        BindBody::Unlowered => Vec::new(),
    }
}

fn param_indices_in_ty(
    ty: &Ty,
    name_to_index: &HashMap<Intern<String>, usize>,
    inferred_index: &HashMap<u32, usize>,
    dependent_binder: BinderId,
) -> Vec<usize> {
    let mut indices = Vec::new();
    walk_param_indices_in_ty(
        ty,
        name_to_index,
        inferred_index,
        dependent_binder,
        &mut indices,
    );
    indices
}

fn walk_param_indices_in_ty(
    ty: &Ty,
    name_to_index: &HashMap<Intern<String>, usize>,
    inferred_index: &HashMap<u32, usize>,
    dependent_binder: BinderId,
    indices: &mut Vec<usize>,
) {
    if let Ty::Opaque(name) = ty {
        if name
            .as_str()
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
            && let Some(index) = name_to_index.get(name)
            && !indices.contains(index)
        {
            indices.push(*index);
        }
    }
    let _: ControlFlow<()> = ty.walk_children(&mut |child| {
        if let Some(child_ty) = child.ty() {
            walk_param_indices_in_ty(
                child_ty,
                name_to_index,
                inferred_index,
                dependent_binder,
                indices,
            );
        } else if !matches!(ty, Ty::Named { .. })
            && let TyChild::Constant(expr) | TyChild::Argument(TyArg::Const(expr)) = child
        {
            walk_param_indices_in_normal_expr(
                expr,
                name_to_index,
                inferred_index,
                dependent_binder,
                indices,
            );
        }
        ControlFlow::Continue(())
    });
}

fn walk_param_indices_in_normal_expr(
    expr: &ast::NormalExpr,
    name_to_index: &HashMap<Intern<String>, usize>,
    inferred_index: &HashMap<u32, usize>,
    dependent_binder: BinderId,
    indices: &mut Vec<usize>,
) {
    match expr {
        ast::NormalExpr::Value(_) => {}
        ast::NormalExpr::Var(name) => {
            if let Some(index) = name_to_index.get(name)
                && !indices.contains(index)
            {
                indices.push(*index);
            }
        }
        ast::NormalExpr::Inferred(id) => {
            if id.binder == dependent_binder
                && let Some(index) = inferred_index.get(&id.slot)
                && !indices.contains(index)
            {
                indices.push(*index);
            }
        }
        ast::NormalExpr::Add(lhs, rhs)
        | ast::NormalExpr::Sub(lhs, rhs)
        | ast::NormalExpr::Mul(lhs, rhs) => {
            walk_param_indices_in_normal_expr(
                lhs,
                name_to_index,
                inferred_index,
                dependent_binder,
                indices,
            );
            walk_param_indices_in_normal_expr(
                rhs,
                name_to_index,
                inferred_index,
                dependent_binder,
                indices,
            );
        }
        ast::NormalExpr::TargetQuery { .. } => {}
    }
}

fn infer_expr_effects(
    typed: &TypedFileAst,
    expr_id: crate::typed::ExprId,
    signatures: &HashMap<DefId, TypedCallableSignature>,
    has_runtime_call: &mut bool,
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
        TypedExprKind::Asm {
            state, operands, ..
        } => {
            if let crate::assembly::AssemblyState::Checked(assembly) = state {
                effects.unknown_memory |= assembly.evaluation.is_none();
                *has_runtime_call |= assembly.evaluation.is_none();
                if let Some(method) = assembly.evaluation
                    && let Some(signature) = signatures.get(&method)
                {
                    effects.unknown_memory |= has_runtime_effects(&signature.effects);
                    *has_runtime_call |= signature.call_capability == CallCapability::RuntimeOnly;
                }
                for (pointer, length) in assembly.memory.reads.iter() {
                    if let Some((_, operand)) =
                        operands.iter().find(|(name, _)| name.as_str() == pointer)
                    {
                        if typed.exprs.ty_of(*operand).is_some_and(Ty::is_ref) {
                            add_exact_effect(typed, *operand, &mut effects.reads);
                        }
                        effects
                            .assembly_reads
                            .push((pointer.clone(), length.clone()));
                    }
                }
                for (pointer, length) in assembly.memory.writes.iter() {
                    if let Some((_, operand)) =
                        operands.iter().find(|(name, _)| name.as_str() == pointer)
                    {
                        if typed.exprs.ty_of(*operand).is_some_and(Ty::is_ref) {
                            add_exact_effect(typed, *operand, &mut effects.writes);
                            add_exact_effect(typed, *operand, &mut effects.invalidates);
                        }
                        effects
                            .assembly_writes
                            .push((pointer.clone(), length.clone()));
                    }
                }
            }
        }
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
        TypedExprKind::AggregateSet { base, index, .. } => {
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
        TypedExprKind::FnCall {
            target,
            args: Some(args),
            ..
        } => {
            let signature = signatures.get(target);
            if let Some(signature) = signature {
                *has_runtime_call |= signature.call_capability == CallCapability::RuntimeOnly;
                apply_call_effects(typed, args, signature, effects);
            }
        }
        _ => {}
    }

    let _ = walk_expr_children(kind, &mut |child| {
        infer_expr_effects(typed, child, signatures, has_runtime_call, effects, visited);
        std::ops::ControlFlow::Continue(())
    });
}

fn apply_call_effects(
    typed: &TypedFileAst,
    args: &[crate::typed::ExprId],
    signature: &TypedCallableSignature,
    effects: &mut FunctionEffects,
) {
    effects.unknown_memory |= signature.effects.unknown_memory;
    effects
        .assembly_reads
        .extend(signature.effects.assembly_reads.iter().cloned());
    effects
        .assembly_writes
        .extend(signature.effects.assembly_writes.iter().cloned());
    for (index, convention) in signature.param_conventions.iter().enumerate() {
        let Some(arg) = args.get(index) else {
            continue;
        };
        match convention {
            ParamConvention::Observe => add_exact_effect(typed, *arg, &mut effects.reads),
            ParamConvention::Mutate => add_exact_effect(typed, *arg, &mut effects.writes),
            ParamConvention::Terminal => {}
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
            let Some(actual) = typed.exprs.target_group_of(*arg).and_then(Option::as_ref) else {
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
        .target_group_of(expr_id)
        .and_then(Option::as_ref)
    else {
        return;
    };
    for target in target.iter() {
        if let Some(target) = EffectTarget::from_reference(target) {
            effects.insert(target);
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
        .target_group_of(expr_id)
        .and_then(Option::as_ref)
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
    let ty = typed.exprs.ty.get(base.as_usize())?;
    let ty =
        reference_pointee_for_type(ty, Some(&typed.type_registry)).unwrap_or_else(|| ty.clone());
    let Ty::Record { fields, .. } = typed.type_registry.resolved_definition_for_type(&ty) else {
        return None;
    };
    fields.iter().position(|(name, _)| *name == field)
}
#[cfg(test)]
mod tests {
    use super::*;
    use ast::ty::{NamedTypeInstance, TypeId, UnionVariant};
    use ast::{BinderOwner, DependentArgId, NormalExpr};

    #[test]
    fn parameter_dependencies_keep_order_and_named_constant_pruning() {
        let names = ["element", "length", "other", "inferred", "extra", "ignored"]
            .map(Intern::from_ref);
        let indices = names
            .iter()
            .copied()
            .enumerate()
            .map(|(index, name)| (name, index))
            .collect();
        let binder = BinderId::new(0, BinderOwner::Expression(0));
        let mut variant = UnionVariant::new(
            names[0],
            vec![
                (
                    names[0],
                    Box::new(Ty::Named {
                        name: names[0],
                        instance: NamedTypeInstance::new(TypeId {
                            file: 0,
                            name: names[0],
                        })
                        .with_arguments(vec![
                            (names[0], TyArg::Type(Box::new(Ty::Opaque(names[0])))),
                            (names[5], TyArg::Const(NormalExpr::Var(names[5]))),
                        ]),
                    }),
                ),
                (
                    names[1],
                    Box::new(Ty::Ref {
                        mutable: false,
                        inner: Box::new(Ty::Array {
                            elem: Box::new(Ty::Opaque(names[0])),
                            size: NormalExpr::Var(names[1]),
                        }),
                    }),
                ),
            ],
        );
        variant.result_ty = Some(Ty::UnresolvedSequence {
            element: Some(Box::new(Ty::Ptr {
                inner: Box::new(Ty::Opaque(names[2])),
            })),
            length: NormalExpr::Inferred(DependentArgId::new(binder, 7)),
        });
        let ty = Ty::Union {
            name: names[0],
            variants: vec![variant],
            literal_values: None,
            resolved_params: Some(vec![
                (names[1], TyArg::Const(NormalExpr::Var(names[1]))),
                (
                    names[4],
                    TyArg::Type(Box::new(Ty::Record {
                        name: names[4],
                        fields: vec![],
                        resolved_params: Some(vec![(
                            names[4],
                            TyArg::Const(NormalExpr::Var(names[4])),
                        )]),
                    })),
                ),
            ]),
        };
        assert_eq!(
            param_indices_in_ty(&ty, &indices, &HashMap::from([(7, 3)]), binder),
            [0, 1, 2, 3, 4]
        );
    }
}
