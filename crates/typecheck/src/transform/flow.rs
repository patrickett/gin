//! Stage 3: Flow — Compute flow contexts, attach flow flaws.
//!
//! This stage walks the typed expression arena depth-first, computes
//! `FlowContext` at each program point, and attaches flow-related flaws
//! (ownership, bounds, narrowing, lin-value checks, etc.).

use std::collections::HashMap;

use crate::analysis::{FlowContext, TyCopyExt, VarState};
use crate::compile_time_trait::CompileTimeTraitRegistry;
use crate::solver::{ConstraintEnv, ProveResult, predicate_expr_to_predicate};
use ast::expr::Literal;
use ast::normal_expr::NormalExpr;
use ast::{ConstValue, ParamConvention};
use diagnostic::Diagnostic;

use crate::ty::Ty;
use crate::typed::{
    DefId, ExprId, GroupId, Overlap, ReferenceTargetGroup, ReferenceTargetSet, TargetIndex,
    TypedCallableSignature, TypedExprKind, TypedFileAst, TypedLoopKind, TypedWhenArm, VariantId,
};
use internment::Intern;

use super::effects::applied_effect_targets;
use super::lower_exprs::target_index_for_expr;

/// Build a map from local bind variable names to their resolved types
/// by scanning the typed expression arena for `TypedExprKind::Bind` entries.
fn collect_local_var_types(typed: &TypedFileAst) -> HashMap<Intern<String>, Ty> {
    let mut var_types = HashMap::new();
    for bind in typed.defs.values() {
        if bind.unassigned_decl {
            var_types.insert(bind.name, bind.return_type.clone());
        }
    }
    for (idx, kind) in typed.exprs.kind.iter().enumerate() {
        if let TypedExprKind::Bind { name, .. } = kind {
            let ty = &typed.exprs.ty[idx];
            var_types.insert(*name, ty.clone());
        }
    }
    var_types
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceValidity {
    Valid,
    PermanentlyInvalid,
    MaybeInvalid,
}

#[derive(Clone)]
pub struct RefTracker {
    targets: HashMap<Intern<String>, ReferenceTargetSet>,
    validity: HashMap<Intern<String>, ReferenceValidity>,
}

impl RefTracker {
    fn new() -> Self {
        Self {
            targets: HashMap::new(),
            validity: HashMap::new(),
        }
    }

    fn register(&mut self, reference: Intern<String>, target: Option<ReferenceTargetSet>) {
        if let Some(target) = target {
            self.targets.insert(reference, target);
            self.validity.insert(reference, ReferenceValidity::Valid);
        }
    }

    fn replace(&mut self, place: &ReferenceTargetGroup, flow_ctx: &mut FlowContext) {
        self.invalidate_descendants(&ReferenceTargetSet::singleton(place.clone()), flow_ctx);
    }

    fn invalidate_descendants(&mut self, sources: &ReferenceTargetSet, flow_ctx: &mut FlowContext) {
        for (reference, targets) in &self.targets {
            if targets.iter().any(|target| {
                sources.iter().any(|source| {
                    if target.is_strict_descendant_of(source, &flow_ctx.constraint_env) {
                        return true;
                    }
                    if source.is_strict_descendant_of(target, &flow_ctx.constraint_env) {
                        return false;
                    }
                    matches!(
                        target.overlap(source, &flow_ctx.constraint_env),
                        Overlap::Partial | Overlap::Unknown
                    )
                })
            }) {
                self.validity
                    .insert(*reference, ReferenceValidity::PermanentlyInvalid);
                flow_ctx.set_var_state(*reference, VarState::Invalidated);
            }
        }
    }

    fn merge_branch_validity(&mut self, branches: &[Self]) {
        for (reference, validity) in &mut self.validity {
            let states: Vec<_> = branches
                .iter()
                .filter_map(|branch| branch.validity.get(reference).copied())
                .collect();
            if states
                .iter()
                .all(|state| *state == ReferenceValidity::PermanentlyInvalid)
            {
                *validity = ReferenceValidity::PermanentlyInvalid;
            } else if states
                .iter()
                .any(|state| *state != ReferenceValidity::Valid)
            {
                *validity = ReferenceValidity::MaybeInvalid;
            }
        }
    }

    fn permanently_invalidate(&mut self, sources: &ReferenceTargetSet, flow_ctx: &mut FlowContext) {
        for (reference, targets) in &self.targets {
            if targets.iter().any(|target| {
                sources.iter().any(|source| {
                    target.overlap(source, &flow_ctx.constraint_env) != Overlap::Disjoint
                })
            }) {
                self.validity
                    .insert(*reference, ReferenceValidity::PermanentlyInvalid);
                flow_ctx.set_var_state(*reference, VarState::Invalidated);
            }
        }
    }
}

/// Walks the expression arena, computes flow contexts at each program point,
/// and attaches flow-related flaws to expressions.
pub fn stage_flow(
    typed: &mut TypedFileAst,
    trait_registry: &CompileTimeTraitRegistry,
    cross_file_signatures: &HashMap<DefId, TypedCallableSignature>,
) {
    // Collect root expression IDs from all bind bodies.
    let mut def_ids: Vec<_> = typed.defs.keys().copied().collect();
    def_ids.sort_by_key(|def_id| {
        let bind = typed.defs.get(def_id);
        let is_value = bind.is_some_and(|bind| bind.params.is_empty());
        (!is_value, def_id.0.as_str().to_string())
    });
    let bind_bodies: Vec<(DefId, Vec<ExprId>)> = def_ids
        .into_iter()
        .filter_map(|def_id| {
            let bind = typed.defs.get(&def_id)?;
            let ids = match &bind.body {
                crate::typed::BindBody::Expr(eid) => vec![*eid],
                crate::typed::BindBody::Body { exprs, ret } => {
                    let mut v: Vec<ExprId> = exprs.clone();
                    v.extend(ret);
                    v
                }
                crate::typed::BindBody::Extern => return None,
            };
            Some((def_id, ids))
        })
        .collect();

    let local_var_types = collect_local_var_types(typed);
    let mut callable_signatures = cross_file_signatures.clone();
    callable_signatures.extend(
        typed
            .defs
            .iter()
            .map(|(def_id, bind)| (*def_id, TypedCallableSignature::from(bind))),
    );

    // Track constants detected at compile time.
    let mut const_bindings: HashMap<Intern<String>, bool> = HashMap::new();

    // Process each bind body.
    for (def_id, root_ids) in &bind_bodies {
        let owned_params: Vec<Intern<String>> = typed
            .defs
            .get(def_id)
            .map(|bind| {
                bind.params
                    .iter()
                    .zip(&bind.param_conventions)
                    .filter_map(|((name, ty), convention)| {
                        (*convention == ParamConvention::Own
                            && !ty.is_copyable(trait_registry, typed))
                        .then_some(*name)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let return_ids: Vec<ExprId> = typed
            .defs
            .get(def_id)
            .map(|bind| match &bind.body {
                crate::typed::BindBody::Expr(id) => vec![*id],
                crate::typed::BindBody::Body { ret: Some(id), .. } => vec![*id],
                crate::typed::BindBody::Body { ret: None, .. } | crate::typed::BindBody::Extern => {
                    vec![]
                }
            })
            .unwrap_or_default();
        let parameter_constraints: Vec<_> = typed
            .defs
            .get(def_id)
            .map(|bind| {
                bind.params
                    .iter()
                    .zip(&bind.param_refinements)
                    .filter_map(|((name, _), refinement)| {
                        refinement
                            .as_ref()
                            .map(|refinement| (*name, refinement.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut flow_ctx = FlowContext::new();
        for (name, refinement) in parameter_constraints {
            let predicate =
                predicate_expr_to_predicate(&refinement, NormalExpr::Var(name), &HashMap::new());
            flow_ctx.constraint_env.assume(&predicate);
        }
        for name in &owned_params {
            flow_ctx.set_var_state(*name, VarState::Alive);
        }
        let mut local_const_values: HashMap<Intern<String>, ConstValue> = HashMap::new();
        let mut ref_tracker = RefTracker::new();
        let mut ctx = WalkExprCtx {
            typed,
            flow_ctx: &mut flow_ctx,
            local_var_types: &local_var_types,
            ref_tracker: &mut ref_tracker,
            const_bindings: &mut const_bindings,
            local_const_values: &mut local_const_values,
            trait_registry,
            callable_signatures: &callable_signatures,
        };
        for root_id in root_ids {
            ctx.walk(*root_id);
        }
        for return_id in return_ids {
            mark_consumed_if_local_ref(ctx.typed, return_id, ctx.flow_ctx);
        }
        emit_unassigned_bindings(ctx.typed, ctx.flow_ctx);
        let unconsumed: Vec<_> = owned_params
            .iter()
            .copied()
            .filter(|name| !matches!(ctx.flow_ctx.get_var_state(name), Some(VarState::Consumed)))
            .collect();
        if let Some(bind) = ctx.typed.defs.get_mut(def_id) {
            for name in unconsumed {
                bind.flaws.push(
                    Diagnostic::new(
                        "type-owned-param-not-consumed",
                        format!("owned parameter `{}` is not returned or consumed", name),
                    )
                    .with_arg("name", name.as_str().to_string()),
                );
            }
        }
    }

    // Process top-level expressions.
    let mut flow_ctx = FlowContext::new();
    let mut local_const_values: HashMap<Intern<String>, ConstValue> = HashMap::new();
    let mut ref_tracker = RefTracker::new();
    let root_ids = typed.root_exprs.clone();
    let mut ctx = WalkExprCtx {
        typed,
        flow_ctx: &mut flow_ctx,
        local_var_types: &local_var_types,
        ref_tracker: &mut ref_tracker,
        const_bindings: &mut const_bindings,
        local_const_values: &mut local_const_values,
        trait_registry,
        callable_signatures: &callable_signatures,
    };
    for root_id in &root_ids {
        ctx.walk(*root_id);
    }
}

fn emit_unassigned_bindings(typed: &mut TypedFileAst, flow_ctx: &FlowContext) {
    let unassigned: Vec<(usize, Intern<String>)> = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .filter_map(|(idx, kind)| match kind {
            TypedExprKind::Bind {
                name,
                unassigned: true,
                ..
            } if matches!(flow_ctx.get_var_state(name), Some(VarState::Declared)) => {
                Some((idx, *name))
            }
            _ => None,
        })
        .collect();
    for (idx, name) in unassigned {
        if !typed.exprs.flaws[idx].iter().any(|f| {
            f.code.slug() == "type-unassigned-binding" && f.arg("name") == Some(name.as_str())
        }) {
            typed.exprs.flaws[idx].push(
                Diagnostic::new(
                    "type-unassigned-binding",
                    format!("binding `{}` is declared but never assigned", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            );
        }
    }
}

fn record_field_index(typed: &TypedFileAst, base: ExprId, field: Intern<String>) -> Option<usize> {
    let mut ty = typed.exprs.ty.get(base.as_usize())?;
    if let Ty::Ref { inner, .. } = ty {
        ty = inner;
    }
    let Ty::Record { fields, .. } = ty else {
        return None;
    };
    fields.iter().position(|(name, _)| *name == field)
}

fn validate_call_refinements(
    typed: &TypedFileAst,
    args: &[ExprId],
    signature: &TypedCallableSignature,
    constraints: &ConstraintEnv,
) -> Vec<Diagnostic> {
    let mut substitutions = HashMap::new();
    for ((name, formal_ty), arg) in signature.params.iter().zip(args) {
        if let TargetIndex::Symbolic(actual) = target_index_for_expr(typed, *arg) {
            substitutions.insert(*name, actual);
        }
        if let Some(actual_ty) = typed.exprs.ty.get(arg.as_usize()) {
            collect_dependent_substitutions(formal_ty, actual_ty, &mut substitutions);
        }
    }

    let mut flaws = Vec::new();
    for (((name, _), refinement), arg) in signature
        .params
        .iter()
        .zip(&signature.param_refinements)
        .zip(args)
    {
        let Some(refinement) = refinement else {
            continue;
        };
        let TargetIndex::Symbolic(actual) = target_index_for_expr(typed, *arg) else {
            flaws.push(
                Diagnostic::new(
                    "type-parameter-refinement-unproven",
                    format!("cannot prove refinement for argument `{}`", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            );
            continue;
        };
        let predicate = predicate_expr_to_predicate(refinement, actual, &substitutions);
        match constraints.prove(&predicate, &HashMap::new()) {
            ProveResult::Proven => {}
            ProveResult::Disproven => flaws.push(
                Diagnostic::new(
                    "type-parameter-refinement-failed",
                    format!(
                        "argument `{}` does not satisfy its refinement",
                        name.as_str()
                    ),
                )
                .with_arg("name", name.as_str().to_string()),
            ),
            ProveResult::Unknown => flaws.push(
                Diagnostic::new(
                    "type-parameter-refinement-unproven",
                    format!("cannot prove refinement for argument `{}`", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            ),
        }
    }
    flaws
}

fn collect_dependent_substitutions(
    formal: &Ty,
    actual: &Ty,
    substitutions: &mut HashMap<Intern<String>, NormalExpr>,
) {
    match (formal, actual) {
        (Ty::Ref { inner: formal, .. }, Ty::Ref { inner: actual, .. }) => {
            collect_dependent_substitutions(formal, actual, substitutions)
        }
        (Ty::Ref { inner: formal, .. }, actual) => {
            collect_dependent_substitutions(formal, actual, substitutions)
        }
        (formal, Ty::Ref { inner: actual, .. }) => {
            collect_dependent_substitutions(formal, actual, substitutions)
        }
        (
            Ty::Array {
                elem: formal_elem,
                size: formal_size,
            },
            Ty::Array {
                elem: actual_elem,
                size: actual_size,
            },
        ) => {
            if let NormalExpr::Var(name) = formal_size {
                substitutions.insert(*name, actual_size.clone());
            }
            collect_dependent_substitutions(formal_elem, actual_elem, substitutions);
        }
        _ => {}
    }
}

fn mark_consumed_if_local_ref(typed: &TypedFileAst, expr_id: ExprId, flow_ctx: &mut FlowContext) {
    let idx = expr_id.as_usize();
    if idx >= typed.exprs.kind.len() {
        return;
    }
    match &typed.exprs.kind[idx] {
        TypedExprKind::Bind { name, .. } => {
            flow_ctx.set_var_state(*name, VarState::Consumed);
        }
        TypedExprKind::FnCall { target, args, .. }
            if args.as_ref().is_none_or(|a| a.is_empty()) =>
        {
            flow_ctx.set_var_state(target.0, VarState::Consumed);
        }
        _ => {}
    }
}

fn should_consume_when_subject(
    typed: &TypedFileAst,
    trait_registry: &CompileTimeTraitRegistry,
    expr_id: ExprId,
) -> bool {
    let Some(ty) = typed.exprs.ty.get(expr_id.as_usize()) else {
        return false;
    };
    !ty.is_ref() && !ty.is_copyable(trait_registry, typed)
}

/// Shared context threaded through flow analysis expression walking.
#[allow(dead_code)]
struct WalkExprCtx<'a> {
    typed: &'a mut TypedFileAst,
    flow_ctx: &'a mut FlowContext,
    local_var_types: &'a HashMap<Intern<String>, Ty>,
    ref_tracker: &'a mut RefTracker,
    const_bindings: &'a mut HashMap<Intern<String>, bool>,

    local_const_values: &'a mut HashMap<Intern<String>, ConstValue>,
    trait_registry: &'a CompileTimeTraitRegistry,
    callable_signatures: &'a HashMap<DefId, TypedCallableSignature>,
}

impl<'a> WalkExprCtx<'a> {
    fn walk_isolated_branches(&mut self, branches: &[Vec<ExprId>]) {
        let incoming_flow = self.flow_ctx.clone();
        let incoming_refs = self.ref_tracker.clone();
        let incoming_bindings = self.const_bindings.clone();
        let incoming_values = self.local_const_values.clone();
        let mut branch_flows = Vec::with_capacity(branches.len());
        let mut branch_refs = Vec::with_capacity(branches.len());

        for branch in branches {
            *self.flow_ctx = incoming_flow.clone();
            *self.ref_tracker = incoming_refs.clone();
            *self.const_bindings = incoming_bindings.clone();
            *self.local_const_values = incoming_values.clone();
            for expr in branch {
                self.walk(*expr);
            }
            branch_flows.push(self.flow_ctx.clone());
            branch_refs.push(self.ref_tracker.clone());
        }

        *self.flow_ctx = incoming_flow;
        self.flow_ctx.merge_branch_states(&branch_flows);
        *self.ref_tracker = incoming_refs;
        self.ref_tracker.merge_branch_validity(&branch_refs);
        *self.const_bindings = incoming_bindings;
        *self.local_const_values = incoming_values;
    }

    fn walk_repeating_body(&mut self, body: &[ExprId], backedge: Option<ExprId>) {
        let incoming_flow = self.flow_ctx.clone();
        let incoming_refs = self.ref_tracker.clone();
        let incoming_bindings = self.const_bindings.clone();
        let incoming_values = self.local_const_values.clone();

        for expr in body {
            self.walk(*expr);
        }
        let first_flow = self.flow_ctx.clone();
        let first_refs = self.ref_tracker.clone();

        if let Some(backedge) = backedge {
            self.walk(backedge);
        }
        for expr in body {
            self.walk(*expr);
        }

        *self.flow_ctx = incoming_flow.clone();
        self.flow_ctx
            .merge_branch_states(&[incoming_flow, first_flow]);
        *self.ref_tracker = incoming_refs.clone();
        self.ref_tracker
            .merge_branch_validity(&[incoming_refs, first_refs]);
        *self.const_bindings = incoming_bindings;
        *self.local_const_values = incoming_values;
    }

    fn walk(&mut self, expr_id: ExprId) {
        let idx = expr_id.as_usize();
        if idx >= self.typed.exprs.kind.len() {
            return;
        }

        // Save context for this expression
        self.typed.exprs.const_value[idx] = self.typed.exprs.const_value[idx].clone();

        match self.typed.exprs.kind[idx].clone() {
            TypedExprKind::Lit(_) => {}
            TypedExprKind::Binary { lhs, rhs, .. } => {
                self.walk(lhs);
                self.walk(rhs);
            }
            TypedExprKind::FnCall { target, args, .. } => {
                if args.as_ref().is_none_or(|a| a.is_empty())
                    && let Some(cv) = self.local_const_values.get(&target.0).cloned()
                {
                    self.typed.exprs.const_value[idx] = Some(cv);
                }
                if args.as_ref().is_none_or(|a| a.is_empty()) {
                    match self.flow_ctx.get_var_state(&target.0) {
                        Some(VarState::Consumed) => {
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-of-moved-value",
                                    format!("use of moved value `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        Some(VarState::MaybeConsumed) => {
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-of-maybe-moved-value",
                                    format!(
                                        "value `{}` may have been moved on another control-flow path",
                                        target.0.as_str()
                                    ),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        Some(VarState::Invalidated) => {
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-of-invalidated-reference",
                                    format!("use of invalidated reference `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        Some(VarState::MaybeInvalidated) => {
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-of-maybe-invalidated-reference",
                                    format!(
                                        "reference `{}` may have been invalidated on another control-flow path",
                                        target.0.as_str()
                                    ),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        Some(VarState::Declared) => {
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-before-assign",
                                    format!("use before assign `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        _ => {}
                    }
                }
                if let Some(arg_ids) = args {
                    let signature = self.callable_signatures.get(&target).cloned();
                    let conventions = signature
                        .as_ref()
                        .map(|signature| signature.param_conventions.clone())
                        .unwrap_or_default();
                    for (param_idx, arg_id) in arg_ids.iter().enumerate() {
                        self.walk(*arg_id);
                        let convention = conventions.get(param_idx);
                        if convention != Some(&ParamConvention::Own) {
                            continue;
                        }
                        let arg_idx = arg_id.as_usize();
                        if matches!(
                            self.typed.exprs.kind.get(arg_idx),
                            Some(TypedExprKind::Ref(_) | TypedExprKind::ConsumeArg(_))
                        ) {
                            continue;
                        }
                        let is_copyable = self
                            .typed
                            .exprs
                            .ty
                            .get(arg_idx)
                            .is_some_and(|ty| ty.is_copyable(self.trait_registry, self.typed));
                        if !is_copyable {
                            mark_consumed_if_local_ref(self.typed, *arg_id, self.flow_ctx);
                            if let Some(source) =
                                self.typed.exprs.target_group[arg_id.as_usize()].clone()
                            {
                                self.ref_tracker
                                    .permanently_invalidate(&source, self.flow_ctx);
                            }
                        }
                    }
                    if let Some(signature) = signature {
                        let refinement_flaws = validate_call_refinements(
                            self.typed,
                            &arg_ids,
                            &signature,
                            &self.flow_ctx.constraint_env,
                        );
                        self.typed.exprs.flaws[idx].extend(refinement_flaws);
                        let application = self.typed.apply_target_groups(&arg_ids, &signature);
                        for (group, ancestor) in application
                            .invalid_derived_groups(&signature, &self.flow_ctx.constraint_env)
                        {
                            let group_path = signature.groups[group.0 as usize]
                                .path
                                .as_ref()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| format!("group{}", group.0));
                            let ancestor_path = signature.groups[ancestor.0 as usize]
                                .path
                                .as_ref()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| format!("group{}", ancestor.0));
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-invalid-derived-target-group-argument",
                                    format!(
                                        "argument for target group `{group_path}` is not within `{ancestor_path}`"
                                    ),
                                )
                                .with_arg("group", group_path)
                                .with_arg("ancestor", ancestor_path),
                            );
                        }
                        for (left, right) in application
                            .overlapping_mutated_pairs(&signature, &self.flow_ctx.constraint_env)
                        {
                            let group_name = |group: GroupId| {
                                signature
                                    .groups
                                    .get(group.0 as usize)
                                    .and_then(|group| group.path.as_ref())
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| format!("group{}", group.0))
                            };
                            let left_name = group_name(left);
                            let right_name = group_name(right);
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-overlapping-target-group-arguments",
                                    format!(
                                        "arguments for target groups `{left_name}` and `{right_name}` may overlap, but one of the groups is mutated"
                                    ),
                                )
                                .with_arg("left_group", left_name)
                                .with_arg("right_group", right_name),
                            );
                        }
                        for applied in applied_effect_targets(
                            self.typed,
                            &arg_ids,
                            &signature,
                            &signature.effects.consumes,
                        ) {
                            self.ref_tracker.permanently_invalidate(
                                &ReferenceTargetSet::singleton(applied.target),
                                self.flow_ctx,
                            );
                        }
                        for applied in applied_effect_targets(
                            self.typed,
                            &arg_ids,
                            &signature,
                            &signature.effects.invalidates,
                        ) {
                            let targets = ReferenceTargetSet::singleton(applied.target);
                            if applied.descendants {
                                self.ref_tracker
                                    .invalidate_descendants(&targets, self.flow_ctx);
                            } else {
                                self.ref_tracker
                                    .permanently_invalidate(&targets, self.flow_ctx);
                            }
                        }
                    }
                }
            }
            TypedExprKind::TagCall {
                variant_id,
                args,
                field_names,
                ..
            } => {
                if let Some(arg_ids) = &args {
                    for arg_id in arg_ids.iter().copied() {
                        self.walk(arg_id);
                        mark_consumed_if_local_ref(self.typed, arg_id, self.flow_ctx);
                    }
                }
                // Check refinements for this TagCall
                self.check_tag_refinements(&variant_id, args.as_deref(), &field_names);
            }
            TypedExprKind::Bind {
                name,
                stmts,
                body,
                unassigned,
            } => {
                for stmt in stmts {
                    if stmt.as_usize() == idx {
                        continue;
                    }
                    self.walk(stmt);
                }
                if !unassigned && body.as_usize() != idx {
                    self.walk(body);
                }
                // Track bind as a constant if its body has a const_value.
                let bind_const = self.typed.exprs.const_value[body.as_usize()]
                    .clone()
                    .or_else(|| self.typed.exprs.const_value[idx].clone());
                self.typed.exprs.const_value[idx] = bind_const.clone();
                if let Some(cv) = bind_const {
                    self.const_bindings.insert(name, true);
                    self.local_const_values.insert(name, cv);
                } else {
                    self.local_const_values.remove(&name);
                }
                if unassigned {
                    self.flow_ctx.set_var_state(name, VarState::Declared);
                } else {
                    self.flow_ctx.set_var_state(name, VarState::Alive);
                    let is_reference =
                        matches!(
                            self.typed.exprs.kind.get(body.as_usize()),
                            Some(TypedExprKind::Ref(_))
                        ) || matches!(self.typed.exprs.ty.get(idx), Some(Ty::Ref { .. }));
                    if is_reference {
                        let targets = self.typed.exprs.target_group[body.as_usize()].clone();
                        self.ref_tracker.register(name, targets);
                    }
                }
            }
            TypedExprKind::Reassign { name, value } => {
                self.walk(value);
                self.ref_tracker
                    .replace(&ReferenceTargetGroup::Local(name), self.flow_ctx);
                self.flow_ctx.set_var_state(name, VarState::Alive);
            }
            TypedExprKind::Eat(inner) => {
                self.walk(inner);
                mark_consumed_if_local_ref(self.typed, inner, self.flow_ctx);
                if let Some(source) = self.typed.exprs.target_group[inner.as_usize()].clone() {
                    self.ref_tracker
                        .permanently_invalidate(&source, self.flow_ctx);
                }
            }
            TypedExprKind::TakePtr(inner) | TypedExprKind::Ref(inner) => {
                self.walk(inner);
            }
            TypedExprKind::When(when_expr) => {
                if let Some(subject_id) = when_expr.subject {
                    self.walk(subject_id);
                    if should_consume_when_subject(self.typed, self.trait_registry, subject_id) {
                        mark_consumed_if_local_ref(self.typed, subject_id, self.flow_ctx);
                    }
                }
                let subject_value = when_expr.subject.and_then(|subject_id| {
                    self.typed.exprs.const_value[subject_id.as_usize()].clone()
                });
                let mut selected_body = None;
                if let Some(subject_value) = &subject_value {
                    for arm in &when_expr.arms {
                        match arm {
                            TypedWhenArm::Is { pattern, body, .. } => {
                                let matches =
                                    crate::analysis::pattern::pattern_matches_with_tag_types(
                                        &pattern.value,
                                        subject_value,
                                        &self.typed.tag_types,
                                    ) || super::lower_exprs::pattern_value_const(
                                        &pattern.value,
                                        self.typed,
                                    )
                                    .as_ref()
                                    .is_some_and(|cv| cv == subject_value);
                                if matches {
                                    selected_body = Some(*body);
                                    break;
                                }
                                self.typed.exprs.flaws[idx].push(Diagnostic::new(
                                    "type-unreachable-when-arm",
                                    "when arm is unreachable",
                                ));
                            }
                            TypedWhenArm::Else(body, _) => {
                                selected_body = Some(*body);
                                break;
                            }
                            TypedWhenArm::Cond { .. } => {}
                        }
                    }
                }
                if let Some(body) = selected_body {
                    self.walk(body);
                    self.typed.exprs.const_value[idx] =
                        self.typed.exprs.const_value[body.as_usize()].clone();
                } else {
                    let branches: Vec<_> = when_expr
                        .arms
                        .iter()
                        .map(|arm| match arm {
                            TypedWhenArm::Cond {
                                condition, body, ..
                            } => vec![*condition, *body],
                            TypedWhenArm::Is { body, .. } | TypedWhenArm::Else(body, _) => {
                                vec![*body]
                            }
                        })
                        .collect();
                    self.walk_isolated_branches(&branches);
                }
            }
            TypedExprKind::If(if_expr) => {
                let comparison_facts = subject_comparison_facts(
                    &self.typed.exprs.kind,
                    if_expr.subject,
                    self.local_const_values,
                );
                let saved_lt_len = self.flow_ctx.constraint_env.known_lt.len();
                let saved_le_len = self.flow_ctx.constraint_env.known_le.len();
                for (lhs, rhs, is_lt) in &comparison_facts {
                    if *is_lt {
                        self.flow_ctx
                            .constraint_env
                            .known_lt
                            .push((lhs.clone(), rhs.clone()));
                    } else {
                        self.flow_ctx
                            .constraint_env
                            .known_le
                            .push((lhs.clone(), rhs.clone()));
                    }
                }
                self.walk(if_expr.subject);
                let mut taken = if_expr.stmts;
                taken.extend(if_expr.ret);
                self.walk_isolated_branches(&[taken, Vec::new()]);
                // Restore constraint_env so facts don't leak past the if-block
                self.flow_ctx.constraint_env.known_lt.truncate(saved_lt_len);
                self.flow_ctx.constraint_env.known_le.truncate(saved_le_len);
            }
            TypedExprKind::Loop(typed_loop) => {
                let backedge = match typed_loop.kind {
                    TypedLoopKind::While { condition } => {
                        self.walk(condition);
                        Some(condition)
                    }
                    TypedLoopKind::ForIn {
                        variable: _,
                        iterable,
                    } => {
                        self.walk(iterable);
                        None
                    }
                };
                self.walk_repeating_body(&typed_loop.stmts, backedge);
            }
            TypedExprKind::TupleGet { base, .. } => {
                self.walk(base);
            }
            TypedExprKind::Destructure { value, .. } => {
                self.walk(value);
            }
            TypedExprKind::RecordSet { base, field, value } => {
                self.walk(base);
                self.walk(value);
                if let Some(base_targets) = self.typed.exprs.target_group[base.as_usize()].clone()
                    && let Some(index) = record_field_index(self.typed, base, field)
                {
                    for base_target in base_targets.iter() {
                        self.ref_tracker.replace(
                            &ReferenceTargetGroup::Field {
                                base: Box::new(base_target.clone()),
                                index,
                            },
                            self.flow_ctx,
                        );
                    }
                }
            }
            TypedExprKind::TupleSet { base, index, value } => {
                self.walk(base);
                self.walk(value);
                if let Some(base_targets) = self.typed.exprs.target_group[base.as_usize()].clone() {
                    for base_target in base_targets.iter() {
                        self.ref_tracker.replace(
                            &ReferenceTargetGroup::Field {
                                base: Box::new(base_target.clone()),
                                index,
                            },
                            self.flow_ctx,
                        );
                    }
                }
            }
            TypedExprKind::Range { start, end } => {
                self.walk(start);
                self.walk(end);
            }
            TypedExprKind::TupleAlloc { init, .. } => {
                self.walk(init);
            }
            TypedExprKind::TupleLit(items) | TypedExprKind::List(items) => {
                for item_id in items {
                    self.walk(item_id);
                }
            }
            TypedExprKind::BufGet { buf, index } => {
                self.walk(buf);
                self.walk(index);
            }
            TypedExprKind::BufSet { buf, index, value } => {
                self.walk(buf);
                self.walk(index);
                self.walk(value);
                let target_index = super::lower_exprs::target_index_for_expr(self.typed, index);
                if let Some(base_targets) = self.typed.exprs.target_group[buf.as_usize()].clone() {
                    for base_target in base_targets.iter() {
                        self.ref_tracker.replace(
                            &ReferenceTargetGroup::ItemRegion {
                                base: Box::new(base_target.clone()),
                                index: target_index.clone(),
                            },
                            self.flow_ctx,
                        );
                    }
                }
            }
            TypedExprKind::Cast { expr: inner, .. } => {
                self.walk(inner);
            }
            TypedExprKind::ConsumeArg(inner) => {
                self.walk(inner);
                mark_consumed_if_local_ref(self.typed, inner, self.flow_ctx);
                if let Some(source) = self.typed.exprs.target_group[inner.as_usize()].clone() {
                    self.ref_tracker
                        .permanently_invalidate(&source, self.flow_ctx);
                }
            }
            TypedExprKind::Deref(inner) | TypedExprKind::Negate(inner) => {
                self.walk(inner);
            }
            TypedExprKind::FormatString(fs) => {
                for part in &fs.parts {
                    if let ast::FormatPart::Expr(_typed_expr, _) = part {
                        // _typed_expr.value is ast::Expr — not an ExprId, so we skip
                        // walking children here. The FormatString's inner expressions
                        // are untyped and won't have flow context attached.
                    }
                }
            }
            TypedExprKind::Asm(_asm_expr) => {
                // TODO: walk asm operand expressions once they carry ExprId
            }
            TypedExprKind::SelfRef { .. } => {}
        }
    }

    fn check_tag_refinements(
        &mut self,
        variant_id: &VariantId,
        args: Option<&[ExprId]>,
        field_names: &[Intern<String>],
    ) {
        let Some(tag) = self.typed.tags.get(&variant_id.union) else {
            return;
        };
        let refinements = &tag.record_field_refinements;
        if refinements.is_empty() {
            return;
        }
        let Some(arg_ids) = args else {
            return;
        };
        let ordered_fields: Vec<Intern<String>> = match &tag.resolved_ty {
            crate::ty::Ty::Record { fields, .. } => fields.iter().map(|(n, _)| *n).collect(),
            _ => return,
        };
        // Build a map of all field values so refinements can reference sibling fields.
        let all_field_values: HashMap<Intern<String>, NormalExpr> = {
            let mut m = HashMap::new();
            for (j, arg_id) in arg_ids.iter().enumerate() {
                let fname = field_names
                    .get(j)
                    .filter(|n| !n.as_str().is_empty())
                    .copied()
                    .or_else(|| ordered_fields.get(j).copied());
                if let Some(fname) = fname
                    && let Some(value) = expr_to_normal_expr(
                        &self.typed.exprs.kind,
                        &self.typed.exprs.const_value,
                        self.local_const_values,
                        *arg_id,
                    )
                {
                    m.insert(fname, value);
                }
            }
            m
        };
        let span_table = &self.typed.span_table;
        for (i, arg_id) in arg_ids.iter().enumerate() {
            let field_name = field_names
                .get(i)
                .filter(|n| !n.as_str().is_empty())
                .copied()
                .or_else(|| ordered_fields.get(i).copied());
            let Some(field_name) = field_name else {
                continue;
            };
            let Some(refinement) = refinements.get(&field_name) else {
                continue;
            };
            let Some(field_value) = expr_to_normal_expr(
                &self.typed.exprs.kind,
                &self.typed.exprs.const_value,
                self.local_const_values,
                *arg_id,
            ) else {
                continue;
            };
            // Include sibling field values so the refinement can reference them.
            // e.g. for `value Int and < n` on a record field, the refinement
            // references the sibling field `n` — we need both in var_values.
            let mut var_values: HashMap<Intern<String>, NormalExpr> = self
                .local_const_values
                .iter()
                .map(|(k, v)| (*k, NormalExpr::Value(v.clone())))
                .collect();
            var_values.extend(all_field_values.iter().map(|(k, v)| (*k, v.clone())));
            let predicate = predicate_expr_to_predicate(refinement, field_value, &var_values);
            match self.flow_ctx.constraint_env.prove(&predicate, &var_values) {
                ProveResult::Proven => {}
                ProveResult::Disproven => {
                    let span = self.typed.exprs.span[arg_id.as_usize()];
                    self.typed.exprs.flaws[arg_id.as_usize()].push(
                        Diagnostic::new(
                            "dep-refinement-failed",
                            format!(
                                "refinement `{:?}` is false for field `{}`",
                                refinement,
                                field_name.as_str()
                            ),
                        )
                        .at_span_id(span, span_table),
                    );
                }
                ProveResult::Unknown => {
                    let span = self.typed.exprs.span[arg_id.as_usize()];
                    self.typed.exprs.flaws[arg_id.as_usize()].push(
                        Diagnostic::new(
                            "dep-refinement-unproven",
                            format!(
                                "cannot prove refinement `{:?}` for field `{}`",
                                refinement,
                                field_name.as_str()
                            ),
                        )
                        .at_span_id(span, span_table),
                    );
                }
            }
        }
    }
}

fn expr_to_normal_expr(
    kinds: &[TypedExprKind],
    const_values: &[Option<ConstValue>],
    lcv: &HashMap<Intern<String>, ConstValue>,
    expr_id: ExprId,
) -> Option<NormalExpr> {
    let idx = expr_id.as_usize();
    if let Some(cv) = const_values.get(idx).and_then(Clone::clone) {
        return Some(NormalExpr::Value(cv));
    }
    match kinds.get(idx)? {
        TypedExprKind::Lit(lit) => match lit {
            Literal::Int(n) => Some(NormalExpr::Value(ConstValue::Int(*n as i128))),
            Literal::Number(n) => Some(NormalExpr::Value(ConstValue::Int(*n as i128))),
            _ => None,
        },
        TypedExprKind::Negate(inner) => expr_to_normal_expr(kinds, const_values, lcv, *inner)
            .and_then(|value| match value {
                NormalExpr::Value(ConstValue::Int(value)) => {
                    Some(NormalExpr::Value(ConstValue::Int(-value)))
                }
                _ => None,
            }),
        TypedExprKind::Ref(inner)
        | TypedExprKind::TakePtr(inner)
        | TypedExprKind::ConsumeArg(inner)
        | TypedExprKind::Eat(inner)
        | TypedExprKind::Deref(inner) => expr_to_normal_expr(kinds, const_values, lcv, *inner),
        TypedExprKind::FnCall { target, args, .. } if args.is_none() => lcv
            .get(&target.0)
            .cloned()
            .map(NormalExpr::Value)
            .or(Some(NormalExpr::Var(target.0))),
        _ => None,
    }
}

/// Extract comparison facts from an `if` subject expression.
/// Returns `(lhs, rhs, is_lt)` tuples where `is_lt` means `lhs < rhs`
/// and `!is_lt` means `lhs <= rhs`.
///
/// Comparison operators desugar to function calls:
/// - `a < b`  → `lt(a, b)`  → `known_lt.push((a, b))`
/// - `a <= b` → `le(a, b)`  → `known_le.push((a, b))`
/// - `a > b`  → `gt(a, b)`  → `known_lt.push((b, a))`
/// - `a >= b` → `ge(a, b)`  → `known_le.push((b, a))`
fn subject_comparison_facts(
    kinds: &[TypedExprKind],
    subject: ExprId,
    lcv: &HashMap<Intern<String>, ConstValue>,
) -> Vec<(NormalExpr, NormalExpr, bool)> {
    let idx = subject.as_usize();
    let Some(TypedExprKind::FnCall { target, args, .. }) = kinds.get(idx) else {
        return Vec::new();
    };
    let Some(arg_ids) = args.as_ref() else {
        return Vec::new();
    };
    if arg_ids.len() != 2 {
        return Vec::new();
    }
    let op_name = target.0.as_str();
    let is_swapped = matches!(op_name, "gt" | "ge");
    let is_lt = matches!(op_name, "lt" | "gt");
    let lhs_idx = if is_swapped { 1 } else { 0 };
    let rhs_idx = if is_swapped { 0 } else { 1 };

    fn to_expr(
        kinds: &[TypedExprKind],
        lcv: &HashMap<Intern<String>, ConstValue>,
        expr_id: ExprId,
    ) -> Option<NormalExpr> {
        let eidx = expr_id.as_usize();
        match kinds.get(eidx)? {
            TypedExprKind::Lit(lit) => match lit {
                Literal::Int(n) => Some(NormalExpr::Value(ConstValue::Int(*n as i128))),
                Literal::Number(n) => Some(NormalExpr::Value(ConstValue::Int(*n as i128))),
                _ => None,
            },
            TypedExprKind::Ref(inner)
            | TypedExprKind::TakePtr(inner)
            | TypedExprKind::ConsumeArg(inner)
            | TypedExprKind::Eat(inner)
            | TypedExprKind::Deref(inner) => to_expr(kinds, lcv, *inner),
            TypedExprKind::FnCall { target, args, .. } if args.is_none() => {
                let var = target.0;
                if let Some(cv) = lcv.get(&var) {
                    Some(NormalExpr::Value(cv.clone()))
                } else {
                    Some(NormalExpr::Var(var))
                }
            }
            _ => None,
        }
    }

    let lhs = to_expr(kinds, lcv, arg_ids[lhs_idx]);
    let rhs = to_expr(kinds, lcv, arg_ids[rhs_idx]);
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => vec![(lhs, rhs, is_lt)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod reference_validity_tests {
    use super::*;

    fn local(name: &str) -> ReferenceTargetGroup {
        ReferenceTargetGroup::Local(Intern::from_ref(name))
    }

    #[test]
    fn field_replacement_restores_reference_validity() {
        let field = ReferenceTargetGroup::Field {
            base: Box::new(local("record")),
            index: 0,
        };
        let reference = Intern::from_ref("saved");
        let mut tracker = RefTracker::new();
        tracker.register(
            reference,
            Some(ReferenceTargetSet::singleton(field.clone())),
        );

        tracker.replace(&field, &mut FlowContext::new());

        assert_eq!(tracker.validity[&reference], ReferenceValidity::Valid);
    }

    #[test]
    fn permanent_source_invalidation_reaches_descendant_places() {
        let source = local("record");
        let field = ReferenceTargetGroup::Field {
            base: Box::new(source.clone()),
            index: 0,
        };
        let reference = Intern::from_ref("saved");
        let mut tracker = RefTracker::new();
        tracker.register(reference, Some(ReferenceTargetSet::singleton(field)));
        let mut flow = FlowContext::new();

        tracker.permanently_invalidate(&ReferenceTargetSet::singleton(source), &mut flow);

        assert_eq!(
            tracker.validity[&reference],
            ReferenceValidity::PermanentlyInvalid
        );
        assert_eq!(flow.get_var_state(&reference), Some(VarState::Invalidated));
    }
}
