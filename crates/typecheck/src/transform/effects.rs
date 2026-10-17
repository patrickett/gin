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

use super::callable_signatures::CallableSignatureScope;
use super::lower_exprs::target_index_for_expr;
use super::proof_walk::read::{ProofVisitor, walk_normal};

mod queries;
mod recursion;
pub use queries::EffectWork;
pub(super) use queries::{EffectQueries, infer_cached};

pub fn infer_package_full(
    typed_files: &mut [TypedFileAst],
    external_signatures: &HashMap<DefId, TypedCallableSignature>,
) -> EffectWork {
    let _profile = compiler_profile::span(compiler_profile::Phase::Effects);
    let mut work = EffectWork::default();
    let recursive_calls = recursion::recursive_calls(typed_files.iter());
    loop {
        work.checked += typed_files
            .iter()
            .map(|file| file.defs.len())
            .sum::<usize>();
        let signatures = CallableSignatureScope::new(
            external_signatures,
            typed_files.iter().flat_map(|typed| {
                typed
                    .defs
                    .iter()
                    .map(|(def_id, bind)| (*def_id, TypedCallableSignature::from(bind)))
            }),
        );

        let updates: Vec<_> = typed_files
            .iter()
            .enumerate()
            .flat_map(|(file_index, typed)| {
                let signatures = &signatures;
                let recursive_calls = &recursive_calls;
                typed.defs.iter().filter_map(move |(def_id, bind)| {
                    let summary =
                        infer_bind_summary(typed, bind, signatures, recursive_calls.get(def_id));
                    (summary.effects != *bind.effects
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
            bind.effects = crate::typed::Shared::new(summary.effects);
            bind.param_requirements = summary.param_requirements;
            bind.call_capability = summary.call_capability;
            bind.definition_stage = summary.definition_stage;
        }
    }
    work
}

#[derive(Debug, Clone)]
pub(super) struct CallableSummary {
    effects: FunctionEffects,
    param_requirements: crate::typed::ParamRequirements,
    call_capability: CallCapability,
    definition_stage: DefinitionStage,
}

fn infer_bind_summary(
    typed: &TypedFileAst,
    bind: &crate::typed::TypedBind,
    signatures: &CallableSignatureScope<'_>,
    recursive_targets: Option<&HashSet<DefId>>,
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
    let mut effects = FunctionEffects {
        postconditions: super::postconditions::common(typed, bind, signatures),
        ..Default::default()
    };
    if bind.unassigned_decl {
        effects.unknown_memory = true;
        for (slot, convention) in bind.param_conventions.iter().enumerate() {
            if *convention == ParamConvention::Mutate {
                if let Some(group) = bind.param_groups.get(slot).copied().flatten() {
                    effects.writes.insert(EffectTarget::Group(group));
                }
            }
        }
    }
    let mut has_runtime_call = false;
    let mut visited = HashSet::new();
    for root in &roots {
        let mut walk = EffectWalk {
            has_runtime_call: &mut has_runtime_call,
            effects: &mut effects,
            visited: &mut visited,
        };
        infer_expr_effects(typed, bind, *root, signatures, &mut walk, recursive_targets);
    }
    for returned in crate::analysis::return_expressions(typed, &bind.body) {
        add_owned_storage_consumption(typed, bind, returned, &mut effects.consumes);
    }
    let param_requirements = infer_param_requirements(bind, typed);
    effects.scalar_fields =
        super::scalar_mutations::infer_fields(typed, bind, signatures, &effects);
    if effects.writes.is_empty()
        && !effects.unknown_memory
        && typed
            .type_registry
            .integer_validity_for_type(&bind.return_type)
            .is_some()
    {
        let result = match &bind.body {
            BindBody::Expr(result) => Some(*result),
            BindBody::Body { exprs, ret } if exprs.is_empty() => *ret,
            _ => None,
        };
        fn uses_parameters(expression: &ast::NormalExpr, bind: &crate::typed::TypedBind) -> bool {
            match expression {
                ast::NormalExpr::Var(name) => bind.params.iter().any(|(parameter, _)| {
                    name == parameter || name.as_str().starts_with(&format!("{parameter}.$field"))
                }),
                ast::NormalExpr::Add(left, right)
                | ast::NormalExpr::Sub(left, right)
                | ast::NormalExpr::Mul(left, right) => {
                    uses_parameters(left, bind) && uses_parameters(right, bind)
                }
                ast::NormalExpr::Value(ast::ConstValue::Int(_))
                | ast::NormalExpr::TargetQuery { .. } => true,
                _ => false,
            }
        }
        effects.scalar_result = result
            .and_then(|result| {
                super::symbolic_values::normal_expr(
                    typed,
                    Some(signatures),
                    &HashMap::new(),
                    result,
                )
            })
            .filter(|result| uses_parameters(result, bind));
    }
    if matches!(
        typed
            .type_registry
            .resolved_definition_for_type(&bind.return_type),
        Ty::Record { .. }
    ) {
        if effects.writes.is_empty() && !effects.unknown_memory {
            let roots = crate::analysis::return_expressions(typed, &bind.body);
            if let [returned] = roots.as_slice()
                && let Some(value @ ast::NormalExpr::Var(_)) = super::symbolic_values::normal_expr(
                    typed,
                    Some(signatures),
                    &HashMap::new(),
                    *returned,
                )
                && let ast::NormalExpr::Var(name) = &value
                && bind
                    .params
                    .iter()
                    .zip(&bind.param_conventions)
                    .any(|((root, _), convention)| {
                        !matches!(
                            convention,
                            ParamConvention::Observe | ParamConvention::Mutate
                        ) && name.as_str().starts_with(&format!("{root}.$field"))
                    })
            {
                effects.scalar_result = Some(value);
            }
        }
        let returned = match &bind.body {
            BindBody::Expr(result) => Some(*result),
            BindBody::Body { ret, .. } => *ret,
            BindBody::Unlowered => None,
        };
        if let Some(returned) = returned
            && let Some(TypedExprKind::FnCall {
                target, args: None, ..
            }) = typed.exprs.kind_of(returned)
            && bind
                .params
                .iter()
                .zip(&bind.param_conventions)
                .any(|((name, ty), convention)| {
                    *name == target.0
                        && *ty == bind.return_type
                        && !matches!(
                            convention,
                            ParamConvention::Observe | ParamConvention::Mutate
                        )
                })
            && visited.iter().all(|expression| {
                *expression == returned
                    || match typed.exprs.kind_of(*expression) {
                        Some(TypedExprKind::If(_)) => false,
                        Some(
                            TypedExprKind::Bind { name, .. } | TypedExprKind::Reassign { name, .. },
                        ) if *name == target.0 => false,
                        Some(TypedExprKind::FnCall {
                            target: other,
                            args: None,
                            ..
                        }) if other == target => false,
                        _ => true,
                    }
            })
        {
            effects.scalar_result = Some(ast::NormalExpr::Var(target.0));
        }
    }
    effects.assembly_reads.sort_unstable();
    effects.assembly_reads.dedup();
    effects.assembly_writes.sort_unstable();
    effects.assembly_writes.dedup();
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
        } else if body_has_compile_time_boundary(typed, &roots, signatures) {
            DefinitionStage::CompileTimeOnly
        } else {
            DefinitionStage::StagePolymorphic
        },
    }
}

fn body_has_compile_time_boundary(
    typed: &TypedFileAst,
    roots: &[crate::typed::ExprId],
    signatures: &CallableSignatureScope<'_>,
) -> bool {
    let mut pending = roots.to_vec();
    let mut visited = HashSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        match typed.exprs.kind_of(id) {
            Some(TypedExprKind::Emit { operands, .. }) => {
                pending.extend(operands.iter().map(|(_, operand)| *operand));
                continue;
            }
            Some(
                TypedExprKind::Domain(_) | TypedExprKind::Target | TypedExprKind::TargetField(_),
            ) => return true,
            Some(TypedExprKind::FnCall { target, args, .. })
                if (args.is_some() || typed.exprs.place_of(id).flatten().is_none())
                    && signatures.get(target).is_some_and(|s| {
                        s.definition_stage == DefinitionStage::CompileTimeOnly
                    }) =>
            {
                return true;
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
) -> crate::typed::ParamRequirements {
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
    if let Ty::Ref { inner, .. } = ty
        && let Ty::Array { elem, .. } = inner.as_ref()
    {
        walk_param_indices_in_ty(
            elem,
            name_to_index,
            inferred_index,
            dependent_binder,
            indices,
        );
        return;
    }
    if let Ty::Opaque(name) = ty
        && name
            .as_str()
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
        && let Some(index) = name_to_index.get(name)
        && !indices.contains(index)
    {
        indices.push(*index);
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
            let _ = ParamIndices {
                name_to_index,
                inferred_index,
                dependent_binder,
                indices,
            }
            .visit_normal(expr);
        }
        ControlFlow::Continue(())
    });
}

struct ParamIndices<'a> {
    name_to_index: &'a HashMap<Intern<String>, usize>,
    inferred_index: &'a HashMap<u32, usize>,
    dependent_binder: BinderId,
    indices: &'a mut Vec<usize>,
}

impl ProofVisitor for ParamIndices<'_> {
    fn visit_normal(&mut self, expr: &ast::NormalExpr) -> ControlFlow<()> {
        match expr {
            ast::NormalExpr::Var(name) => {
                if let Some(index) = self.name_to_index.get(name)
                    && !self.indices.contains(index)
                {
                    self.indices.push(*index);
                }
            }
            ast::NormalExpr::Inferred(id) => {
                if id.binder == self.dependent_binder
                    && let Some(index) = self.inferred_index.get(&id.slot)
                    && !self.indices.contains(index)
                {
                    self.indices.push(*index);
                }
            }
            _ => {}
        }
        walk_normal(self, expr)
    }
}

struct EffectWalk<'a> {
    has_runtime_call: &'a mut bool,
    effects: &'a mut FunctionEffects,
    visited: &'a mut HashSet<crate::typed::ExprId>,
}

fn infer_expr_effects(
    typed: &TypedFileAst,
    definition: &crate::typed::TypedBind,
    expr_id: crate::typed::ExprId,
    signatures: &CallableSignatureScope<'_>,
    walk: &mut EffectWalk<'_>,
    recursive_targets: Option<&HashSet<DefId>>,
) {
    let has_runtime_call = &mut *walk.has_runtime_call;
    let effects = &mut *walk.effects;
    let visited = &mut *walk.visited;
    if !visited.insert(expr_id) {
        return;
    }
    let idx = expr_id.as_usize();
    let Some(kind) = typed.exprs.kind.get(idx) else {
        return;
    };

    add_exact_effect(typed, definition, expr_id, &mut effects.reads);

    let written_base = match kind {
        TypedExprKind::AggregateSet { base, .. } | TypedExprKind::RecordSet { base, .. } => {
            Some(*base)
        }
        TypedExprKind::BufSet { buf, .. } => Some(*buf),
        _ => None,
    };
    if let Some(base) = written_base
        && typed
            .exprs
            .ty_of(base)
            .is_some_and(|ty| reference_pointee_for_type(ty, Some(&typed.type_registry)).is_some())
        && effect_reference_targets(typed, base, definition).is_none()
    {
        effects.unknown_memory = true;
    }
    match kind {
        TypedExprKind::Bind {
            body,
            unassigned: false,
            ..
        } => {
            add_owned_storage_consumption(typed, definition, *body, &mut effects.consumes);
        }
        TypedExprKind::TakePtr(inner) => {
            add_exact_effect(typed, definition, *inner, &mut effects.raw_exposes);
        }
        TypedExprKind::Cast { expr, .. }
            if is_raw_pointer_buffer(typed, expr_id) && typed.borrows_value(*expr) =>
        {
            add_exact_effect(typed, definition, *expr, &mut effects.raw_exposes);
        }
        TypedExprKind::Reassign { .. }
            if typed
                .exprs
                .place_of(expr_id)
                .flatten()
                .is_some_and(|place| {
                    let place = &typed.places[place.0 as usize];
                    place.projection.is_some()
                        || reference_pointee_for_type(&place.ty, Some(&typed.type_registry))
                            .is_none()
                }) =>
        {
            add_exact_effect(typed, definition, expr_id, &mut effects.writes);
            add_exact_effect(typed, definition, expr_id, &mut effects.invalidates);
        }
        TypedExprKind::Emit {
            state: crate::assembly::AssemblyState::Checked(assembly),
            operands,
            ..
        } => {
            effects.unknown_memory |= !assembly.memory.writes.is_empty();
            *has_runtime_call = true;
            for (pointer, length) in assembly.memory.reads.iter() {
                if let Some((_, operand)) =
                    operands.iter().find(|(name, _)| name.as_str() == pointer)
                {
                    if typed.exprs.ty_of(*operand).is_some_and(Ty::is_ref) {
                        add_exact_effect(typed, definition, *operand, &mut effects.reads);
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
                        add_exact_effect(typed, definition, *operand, &mut effects.writes);
                        add_exact_effect(typed, definition, *operand, &mut effects.invalidates);
                    }
                    effects
                        .assembly_writes
                        .push((pointer.clone(), length.clone()));
                }
            }
        }
        TypedExprKind::RecordSet { base, field, .. } => {
            if let Some(index) = record_field_index(typed, *base, *field) {
                add_projected_effect(typed, definition, *base, &mut effects.writes, |base| {
                    EffectTarget::Field {
                        base: Box::new(base),
                        index,
                    }
                });
                add_projected_effect(typed, definition, *base, &mut effects.invalidates, |base| {
                    EffectTarget::Descendants(Box::new(EffectTarget::Field {
                        base: Box::new(base),
                        index,
                    }))
                });
            }
        }
        TypedExprKind::AggregateSet { base, index, .. } => {
            add_projected_effect(typed, definition, *base, &mut effects.writes, |base| {
                EffectTarget::Field {
                    base: Box::new(base),
                    index: *index,
                }
            });
            add_projected_effect(typed, definition, *base, &mut effects.invalidates, |base| {
                EffectTarget::Descendants(Box::new(EffectTarget::Field {
                    base: Box::new(base),
                    index: *index,
                }))
            });
        }
        TypedExprKind::IntrinsicCall {
            op: crate::intrinsic::IntrinsicOp::AddressStore,
            ..
        } => {
            effects.unknown_memory = true;
        }
        TypedExprKind::BufSet { buf, index, .. } => {
            if is_raw_pointer_buffer(typed, *buf) {
                effects.unknown_memory = true;
            }
            let index = target_index_for_expr(typed, *index);
            add_projected_effect(typed, definition, *buf, &mut effects.writes, |base| {
                EffectTarget::ItemRegion {
                    base: Box::new(base),
                    index: index.clone(),
                }
            });
            add_projected_effect(typed, definition, *buf, &mut effects.invalidates, |base| {
                EffectTarget::Descendants(Box::new(EffectTarget::ItemRegion {
                    base: Box::new(base),
                    index: index.clone(),
                }))
            });
        }
        TypedExprKind::FnCall { target, args, .. }
            if args.is_some() || typed.exprs.place_of(expr_id).flatten().is_none() =>
        {
            let signature = signatures.get(target);
            if let Some(signature) = signature {
                *has_runtime_call |= signature.call_capability == CallCapability::RuntimeOnly;
                apply_call_effects(
                    typed,
                    definition,
                    args.as_deref().unwrap_or(&[]),
                    signature,
                    effects,
                    recursive_targets.is_some_and(|targets| targets.contains(target)),
                );
            }
        }
        _ => {}
    }

    let _ = walk_expr_children(kind, &mut |child| {
        infer_expr_effects(
            typed,
            definition,
            child,
            signatures,
            &mut EffectWalk {
                has_runtime_call: &mut *has_runtime_call,
                effects: &mut *effects,
                visited: &mut *visited,
            },
            recursive_targets,
        );
        std::ops::ControlFlow::Continue(())
    });
}

fn apply_call_effects(
    typed: &TypedFileAst,
    definition: &crate::typed::TypedBind,
    args: &[crate::typed::ExprId],
    signature: &TypedCallableSignature,
    effects: &mut FunctionEffects,
    recursive: bool,
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
            ParamConvention::Observe => {
                add_exact_effect(typed, definition, *arg, &mut effects.reads)
            }
            ParamConvention::Mutate => {
                if effect_reference_targets(typed, *arg, definition).is_none() {
                    effects.unknown_memory = true;
                }
            }
            ParamConvention::Terminal | ParamConvention::Own => {
                add_owned_storage_consumption(typed, definition, *arg, &mut effects.consumes);
            }
        }
    }

    substitute_effects(
        typed,
        definition,
        args,
        signature,
        &signature.effects.reads,
        &mut effects.reads,
        recursive,
    );
    substitute_effects(
        typed,
        definition,
        args,
        signature,
        &signature.effects.writes,
        &mut effects.writes,
        recursive,
    );
    substitute_effects(
        typed,
        definition,
        args,
        signature,
        &signature.effects.invalidates,
        &mut effects.invalidates,
        recursive,
    );
    substitute_effects(
        typed,
        definition,
        args,
        signature,
        &signature.effects.consumes,
        &mut effects.consumes,
        recursive,
    );
    substitute_effects(
        typed,
        definition,
        args,
        signature,
        &signature.effects.raw_exposes,
        &mut effects.raw_exposes,
        recursive,
    );
}

fn substitute_effects(
    typed: &TypedFileAst,
    definition: &crate::typed::TypedBind,
    args: &[crate::typed::ExprId],
    signature: &TypedCallableSignature,
    callee_targets: &HashSet<EffectTarget>,
    caller_targets: &mut HashSet<EffectTarget>,
    recursive: bool,
) {
    for applied in applied_effect_targets_with(args, signature, callee_targets, |arg| {
        effect_reference_targets(typed, arg, definition).map(std::borrow::Cow::Owned)
    }) {
        if recursive && let Some(group) = applied.target.root_param() {
            caller_targets.insert(EffectTarget::Descendants(Box::new(EffectTarget::Group(
                group,
            ))));
        } else if let Some(mut target) = EffectTarget::from_reference(&applied.target) {
            if applied.descendants {
                target = EffectTarget::Descendants(Box::new(target));
            }
            caller_targets.insert(target);
        }
    }
}

pub(crate) fn applied_effect_targets_with<'a>(
    args: &[crate::typed::ExprId],
    signature: &TypedCallableSignature,
    effects: &HashSet<EffectTarget>,
    mut targets_for: impl FnMut(
        crate::typed::ExprId,
    ) -> Option<std::borrow::Cow<'a, crate::typed::ReferenceTargetSet>>,
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
            let Some(actual) = targets_for(*arg) else {
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

fn effect_reference_targets(
    typed: &TypedFileAst,
    expression: crate::typed::ExprId,
    definition: &crate::typed::TypedBind,
) -> Option<crate::typed::ReferenceTargetSet> {
    use crate::typed::ReferenceTargetGroup;
    if typed
        .exprs
        .ty_of(expression)
        .is_some_and(|ty| reference_pointee_for_type(ty, Some(&typed.type_registry)).is_some())
    {
        let references = typed.exprs.references_of(expression.index());
        if !references.sources.is_empty() {
            return references.targets_in_definition(definition, &typed.type_registry);
        }
    }
    match typed.exprs.kind_of(expression)? {
        TypedExprKind::Ref(inner) => effect_reference_targets(typed, *inner, definition),
        TypedExprKind::Deref(inner)
            if matches!(
                typed.exprs.kind_of(*inner),
                Some(TypedExprKind::AggregateGet { .. })
            ) =>
        {
            effect_reference_targets(typed, *inner, definition)
        }
        TypedExprKind::AggregateGet { base, index } => {
            if let Some(targets) = typed
                .exprs
                .result_target_groups_of(*base)
                .and_then(|fields| fields.get(*index))
                .and_then(Clone::clone)
            {
                return Some(targets);
            }
            let targets = effect_reference_targets(typed, *base, definition)?.projected(|base| {
                ReferenceTargetGroup::Field {
                    base: Box::new(base.clone()),
                    index: *index,
                }
            });
            if typed.exprs.ty_of(expression).is_some_and(|ty| {
                reference_pointee_for_type(ty, Some(&typed.type_registry)).is_some()
            }) {
                Some(
                    targets
                        .projected(|target| ReferenceTargetGroup::Deref(Box::new(target.clone()))),
                )
            } else {
                Some(targets)
            }
        }
        _ => typed.exprs.target_group_of(expression).cloned().flatten(),
    }
}

fn add_owned_storage_consumption(
    typed: &TypedFileAst,
    definition: &crate::typed::TypedBind,
    expression: crate::ExprId,
    effects: &mut HashSet<EffectTarget>,
) {
    if typed.borrows_value(expression)
        || crate::analysis::expr_is_rematerializable(typed, expression)
    {
        return;
    }
    match typed.exprs.kind_of(expression) {
        Some(TypedExprKind::BufGet { buf, .. } | TypedExprKind::Deref(buf))
            if typed.exprs.ty_of(*buf).is_some_and(|ty| {
                matches!(
                    typed
                        .type_registry
                        .resolved_definition_for_type(ty.without_reference_wrappers()),
                    Ty::OwnedStorage { .. }
                )
            }) =>
        {
            add_exact_effect(typed, definition, expression, effects);
        }
        Some(TypedExprKind::Cast { expr, .. } | TypedExprKind::AggregateGet { base: expr, .. }) => {
            add_owned_storage_consumption(typed, definition, *expr, effects);
        }
        _ => {}
    }
}

fn add_exact_effect(
    typed: &TypedFileAst,
    definition: &crate::typed::TypedBind,
    expr_id: crate::typed::ExprId,
    effects: &mut HashSet<EffectTarget>,
) {
    add_projected_effect(typed, definition, expr_id, effects, |target| target);
}

fn add_projected_effect(
    typed: &TypedFileAst,
    definition: &crate::typed::TypedBind,
    expr_id: crate::typed::ExprId,
    effects: &mut HashSet<EffectTarget>,
    mut project: impl FnMut(EffectTarget) -> EffectTarget,
) {
    let Some(target) = effect_reference_targets(typed, expr_id, definition) else {
        return;
    };
    for target in target.iter() {
        if let Some(target) = EffectTarget::from_reference(target) {
            effects.insert(project(target));
        }
    }
}

pub(super) fn is_raw_pointer_buffer(typed: &TypedFileAst, buffer: crate::ExprId) -> bool {
    typed.exprs.ty_of(buffer).is_some_and(|ty| {
        matches!(
            typed
                .type_registry
                .resolved_definition_for_type(ty.without_reference_wrappers()),
            Ty::Ptr { .. } | Ty::Address { .. }
        )
    })
}

pub(super) fn record_field_index(
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
        let names =
            ["element", "length", "other", "inferred", "extra", "ignored"].map(Intern::from_ref);
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
            [0, 2, 3, 1, 4]
        );
    }
}
