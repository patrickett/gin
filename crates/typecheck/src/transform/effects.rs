use std::collections::{HashMap, HashSet};

use ast::{BinderId, ParamConvention, TyArg};
use internment::Intern;

use crate::ty::Ty;
use crate::typed::{
    AppliedEffectTarget, AvailabilityRequirement, BindBody, CallCapability, DefId, EffectTarget,
    FunctionEffects, TypedCallableSignature, TypedExprKind, TypedFileAst, walk_expr_children,
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
            {
                let typed_bind = typed.defs.get_mut(def_id).unwrap();
                typed_bind.effects = summary.effects;
                typed_bind.param_requirements = summary.param_requirements;
                typed_bind.call_capability = summary.call_capability;
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
                        || summary.call_capability != bind.call_capability)
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
        }
    }
}

#[derive(Debug, Clone)]
struct CallableSummary {
    effects: FunctionEffects,
    param_requirements: Vec<AvailabilityRequirement>,
    call_capability: CallCapability,
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
        BindBody::Extern => Vec::new(),
    };
    let mut effects = FunctionEffects::default();
    let mut has_runtime_call = false;
    let mut visited = HashSet::new();
    for root in roots {
        infer_expr_effects(
            typed,
            root,
            signatures,
            &mut has_runtime_call,
            &mut effects,
            &mut visited,
        );
    }
    let param_requirements = infer_param_requirements(bind, typed);
    let call_capability = if has_runtime_effects(&effects) || has_runtime_call {
        CallCapability::RuntimeOnly
    } else {
        CallCapability::StagePolymorphic
    };

    CallableSummary {
        effects,
        param_requirements,
        call_capability,
    }
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
    !effects.reads.is_empty()
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
        BindBody::Extern => Vec::new(),
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
    match ty {
        Ty::Record {
            fields,
            resolved_params,
            ..
        } => {
            for (_, field_ty) in fields {
                walk_param_indices_in_ty(
                    field_ty,
                    name_to_index,
                    inferred_index,
                    dependent_binder,
                    indices,
                );
            }
            if let Some(params) = resolved_params {
                for (_, arg) in params {
                    match arg {
                        TyArg::Type(inner_ty) => walk_param_indices_in_ty(
                            inner_ty,
                            name_to_index,
                            inferred_index,
                            dependent_binder,
                            indices,
                        ),
                        TyArg::Const(expr) => walk_param_indices_in_normal_expr(
                            expr,
                            name_to_index,
                            inferred_index,
                            dependent_binder,
                            indices,
                        ),
                    }
                }
            }
        }
        Ty::Union {
            variants,
            resolved_params,
            ..
        } => {
            for variant in variants {
                for (_, field_ty) in &variant.fields {
                    walk_param_indices_in_ty(
                        field_ty,
                        name_to_index,
                        inferred_index,
                        dependent_binder,
                        indices,
                    );
                }
                if let Some(result_ty) = &variant.result_ty {
                    walk_param_indices_in_ty(
                        result_ty,
                        name_to_index,
                        inferred_index,
                        dependent_binder,
                        indices,
                    );
                }
            }
            if let Some(params) = resolved_params {
                for (_, arg) in params {
                    match arg {
                        TyArg::Type(inner_ty) => walk_param_indices_in_ty(
                            inner_ty,
                            name_to_index,
                            inferred_index,
                            dependent_binder,
                            indices,
                        ),
                        TyArg::Const(expr) => walk_param_indices_in_normal_expr(
                            expr,
                            name_to_index,
                            inferred_index,
                            dependent_binder,
                            indices,
                        ),
                    }
                }
            }
        }
        Ty::Array { elem, size } => {
            walk_param_indices_in_ty(
                elem,
                name_to_index,
                inferred_index,
                dependent_binder,
                indices,
            );
            walk_param_indices_in_normal_expr(
                size,
                name_to_index,
                inferred_index,
                dependent_binder,
                indices,
            );
        }
        Ty::Ptr { inner } | Ty::Ref { inner, .. } => {
            walk_param_indices_in_ty(
                inner,
                name_to_index,
                inferred_index,
                dependent_binder,
                indices,
            );
        }
        Ty::Tuple(elements) => {
            for element in elements {
                walk_param_indices_in_ty(
                    element,
                    name_to_index,
                    inferred_index,
                    dependent_binder,
                    indices,
                );
            }
        }
        Ty::Int { .. } | Ty::Float { .. } | Ty::Unit | Ty::Literal(_) => {}
        Ty::Opaque(name) => {
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
    }
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
            if target.0.as_str() == "asm" {
                *has_runtime_call = true;
            }
            if let Some(signature) = signature {
                apply_call_effects(typed, args, signature, effects);
            }
        }
        TypedExprKind::Asm(_) => {
            *has_runtime_call = true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::{PackageTransformOptions, TransformCtx, transform, transform_package};
    use crate::typed::FileId;
    use internment::Intern;
    use parser::cursor::TokenCursor;

    fn signature_of(source: &str, name: &str) -> TypedCallableSignature {
        let file_ast = TokenCursor::parse_source(source);
        let typed = transform(&file_ast, FileId(0), &TransformCtx::new());
        let bind = typed
            .defs
            .get(&DefId(Intern::from_ref(name)))
            .expect("bind not found");
        TypedCallableSignature::from(bind)
    }

    #[test]
    fn pure_bind_is_stage_polymorphic() {
        let signature = signature_of("add(x Int, y Int): x + y\n", "add");
        assert_eq!(signature.call_capability, CallCapability::StagePolymorphic);
        assert_eq!(
            signature.param_requirements,
            vec![AvailabilityRequirement::Any, AvailabilityRequirement::Any]
        );
    }

    #[test]
    fn asm_is_runtime_only() {
        let source =
            "spec := 'svc #0x80'\nwrite(fd Int, buf Int, len Int) Int: asm(spec, fd, buf, len)\n";
        let signature = signature_of(source, "write");
        assert_eq!(signature.call_capability, CallCapability::RuntimeOnly);
    }

    #[test]
    fn param_type_reference_marks_requirement_compile_time() {
        let signature = signature_of("make(size Int, y size): 1\n", "make");
        assert_eq!(
            signature.param_requirements,
            vec![
                AvailabilityRequirement::CompileTime,
                AvailabilityRequirement::CompileTime
            ]
        );
    }

    #[test]
    fn runtime_expression_only_param_remains_any() {
        let signature = signature_of("runtime_only(x Int): Int: x + 1\n", "runtime_only");
        assert_eq!(
            signature.param_requirements,
            vec![AvailabilityRequirement::Any]
        );
    }

    #[test]
    fn mutually_recursive_runtime_free_functions_are_stage_polymorphic() {
        let source = "f(x Int): Int: g(x)\ng(x Int): Int: f(x)\n";
        let signature = signature_of(source, "f");
        assert_eq!(signature.call_capability, CallCapability::StagePolymorphic);
    }

    #[test]
    fn cross_file_runtime_calls_set_runtime_capability() {
        let callee =
            TokenCursor::parse_source("Cell has value Int\na(mut cell Cell): b(mut cell)\n");
        let caller = TokenCursor::parse_source(
            "Cell has value Int\nb(mut cell Cell):\n    return eat cell\n",
        );
        let typed = transform_package(
            &[(callee, FileId(0)), (caller, FileId(1))],
            &TransformCtx::new(),
            PackageTransformOptions::IDE,
        );

        let destroy = typed
            .iter()
            .find_map(|typed_file| typed_file.defs.get(&DefId(Intern::from_ref("a"))))
            .expect("definition");
        let caller = typed
            .iter()
            .find_map(|typed_file| typed_file.defs.get(&DefId(Intern::from_ref("b"))))
            .expect("definition");

        assert_eq!(destroy.call_capability, CallCapability::RuntimeOnly);
        assert_eq!(caller.call_capability, CallCapability::RuntimeOnly);
    }
}
