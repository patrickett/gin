//! Stage 3: Flow — Compute flow contexts, attach flow flaws.
//!
//! This stage walks the typed expression arena depth-first, computes
//! `FlowContext` at each program point, and attaches flow-related flaws
//! (ownership, bounds, narrowing, lin-value checks, etc.).

use std::collections::HashMap;

use crate::analysis::{FlowContext, TyCopyExt, VarState};
use crate::compile_time_trait::CompileTimeTraitRegistry;
use crate::reflect::reflect_ty_to_const_value;
use crate::solver::{ConstraintEnv, ProveResult, predicate_expr_to_predicate};
use ast::const_expr::ConstExpr;
use ast::expr::Literal;
use ast::{ConstValue, HashFloat, ParamConvention};
use diagnostic::Diagnostic;

use crate::ty::Ty;
use crate::typed::{
    DefId, ExprId, Overlap, ReferenceTargetGroup, TargetIndex, TypedCallableSignature,
    TypedExprKind, TypedFileAst, TypedLoopKind, TypedWhenArm, VariantId,
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

pub struct RefTracker {
    targets: HashMap<Intern<String>, ReferenceTargetGroup>,
    validity: HashMap<Intern<String>, ReferenceValidity>,
}

impl RefTracker {
    fn new() -> Self {
        Self {
            targets: HashMap::new(),
            validity: HashMap::new(),
        }
    }

    fn register(&mut self, reference: Intern<String>, target: Option<ReferenceTargetGroup>) {
        if let Some(target) = target {
            self.targets.insert(reference, target);
            self.validity.insert(reference, ReferenceValidity::Valid);
        }
    }

    fn replace(&mut self, place: &ReferenceTargetGroup, constraints: &ConstraintEnv) {
        let affected: Vec<_> = self
            .targets
            .iter()
            .filter_map(|(reference, target)| {
                (target.overlap(place, constraints) != Overlap::Disjoint).then_some(*reference)
            })
            .collect();
        for reference in &affected {
            self.validity
                .insert(*reference, ReferenceValidity::MaybeInvalid);
        }
        for reference in affected {
            self.validity.insert(reference, ReferenceValidity::Valid);
        }
    }

    fn permanently_invalidate(
        &mut self,
        source: &ReferenceTargetGroup,
        flow_ctx: &mut FlowContext,
    ) {
        for (reference, target) in &self.targets {
            if target.overlap(source, &flow_ctx.constraint_env) != Overlap::Disjoint {
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

    let mut ref_tracker = RefTracker::new();

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
                predicate_expr_to_predicate(&refinement, ConstExpr::Var(name), &HashMap::new());
            flow_ctx.constraint_env.assume(&predicate);
        }
        for name in &owned_params {
            flow_ctx.set_var_state(*name, VarState::Alive);
        }
        let mut local_const_values: HashMap<Intern<String>, ConstValue> = HashMap::new();
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

fn referenced_source(typed: &TypedFileAst, expr_id: ExprId) -> Option<Intern<String>> {
    match typed.exprs.kind.get(expr_id.as_usize())? {
        TypedExprKind::FnCall { target, args, .. }
            if args.as_ref().is_none_or(|args| args.is_empty()) =>
        {
            Some(target.0)
        }
        TypedExprKind::TupleGet { base, .. } | TypedExprKind::BufGet { buf: base, .. } => {
            referenced_source(typed, *base)
        }
        TypedExprKind::Deref(inner) | TypedExprKind::Ref(inner) => referenced_source(typed, *inner),
        _ => None,
    }
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
    substitutions: &mut HashMap<Intern<String>, ConstExpr>,
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
            if let ConstExpr::Var(name) = formal_size {
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
                        Some(VarState::Invalidated) => {
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-of-invalidated-reference",
                                    format!("use of invalidated reference `{}`", target.0.as_str()),
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
                        for applied in applied_effect_targets(
                            self.typed,
                            &arg_ids,
                            &signature,
                            &signature.effects.invalidates,
                        ) {
                            self.ref_tracker
                                .permanently_invalidate(&applied.target, self.flow_ctx);
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
                    }
                }
                // Check refinements for this TagCall
                self.check_tag_refinements(&variant_id, &args, &field_names);
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
                    let reference_source = match self.typed.exprs.kind.get(body.as_usize()) {
                        Some(TypedExprKind::Ref(inner)) => referenced_source(self.typed, *inner),
                        _ if matches!(self.typed.exprs.ty.get(idx), Some(Ty::Ref { .. })) => {
                            referenced_source(self.typed, body)
                        }
                        _ => None,
                    };
                    if reference_source.is_some() {
                        let target = self.typed.exprs.target_group[body.as_usize()].clone();
                        self.ref_tracker.register(name, target);
                    }
                }
            }
            TypedExprKind::Reassign { name, value } => {
                self.walk(value);
                self.ref_tracker.replace(
                    &ReferenceTargetGroup::Local(name),
                    &self.flow_ctx.constraint_env,
                );
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
                    for arm in &when_expr.arms {
                        match arm {
                            TypedWhenArm::Cond {
                                condition, body, ..
                            } => {
                                self.walk(*condition);
                                self.walk(*body);
                            }
                            TypedWhenArm::Is { body, .. } => {
                                self.walk(*body);
                            }
                            TypedWhenArm::Else(body, _) => {
                                self.walk(*body);
                            }
                        }
                    }
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
                for body_id in if_expr.stmts {
                    self.walk(body_id);
                }
                if let Some(ret_id) = if_expr.ret {
                    self.walk(ret_id);
                }
                // Restore constraint_env so facts don't leak past the if-block
                self.flow_ctx.constraint_env.known_lt.truncate(saved_lt_len);
                self.flow_ctx.constraint_env.known_le.truncate(saved_le_len);
            }
            TypedExprKind::Loop(typed_loop) => {
                match typed_loop.kind {
                    TypedLoopKind::While { condition } => {
                        self.walk(condition);
                    }
                    TypedLoopKind::ForIn {
                        variable: _,
                        iterable,
                    } => {
                        self.walk(iterable);
                    }
                }
                for body_id in typed_loop.stmts {
                    self.walk(body_id);
                }
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
                if let Some(base_target) = self.typed.exprs.target_group[base.as_usize()].clone()
                    && let Some(index) = record_field_index(self.typed, base, field)
                {
                    self.ref_tracker.replace(
                        &ReferenceTargetGroup::Field {
                            base: Box::new(base_target),
                            index,
                        },
                        &self.flow_ctx.constraint_env,
                    );
                }
            }
            TypedExprKind::TupleSet { base, index, value } => {
                self.walk(base);
                self.walk(value);
                if let Some(base_target) = self.typed.exprs.target_group[base.as_usize()].clone() {
                    self.ref_tracker.replace(
                        &ReferenceTargetGroup::Field {
                            base: Box::new(base_target),
                            index,
                        },
                        &self.flow_ctx.constraint_env,
                    );
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
            TypedExprKind::BufGet { buf, index } | TypedExprKind::BufSet { buf, index, .. } => {
                self.walk(buf);
                self.walk(index);
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
        args: &Option<Vec<ExprId>>,
        field_names: &[Intern<String>],
    ) {
        let Some(tag) = self.typed.tags.get(&variant_id.union) else {
            return;
        };
        let refinements = &tag.record_field_refinements;
        if refinements.is_empty() {
            return;
        }
        let Some(arg_ids) = args.as_ref() else {
            return;
        };
        let ordered_fields: Vec<Intern<String>> = match &tag.resolved_ty {
            crate::ty::Ty::Record { fields, .. } => fields.iter().map(|(n, _)| *n).collect(),
            _ => return,
        };
        // Build a map of all field values so refinements can reference sibling fields.
        let all_field_values: HashMap<Intern<String>, ConstExpr> = {
            let mut m = HashMap::new();
            for (j, arg_id) in arg_ids.iter().enumerate() {
                let fname = field_names
                    .get(j)
                    .filter(|n| !n.as_str().is_empty())
                    .copied()
                    .or_else(|| ordered_fields.get(j).copied());
                if let Some(fname) = fname
                    && let Some(value) = expr_const_expr(
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
            let Some(field_value) = expr_const_expr(
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
            let mut var_values: HashMap<Intern<String>, ConstExpr> = self
                .local_const_values
                .iter()
                .map(|(k, v)| (*k, ConstExpr::Value(v.clone())))
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

pub(crate) fn children_of_kind(kind: &TypedExprKind) -> Vec<ExprId> {
    let mut children = Vec::new();
    match kind {
        TypedExprKind::Lit(_) => {}
        TypedExprKind::Binary { lhs, rhs, .. } => {
            children.push(*lhs);
            children.push(*rhs);
        }
        TypedExprKind::FnCall { args, .. } => {
            if let Some(arg_ids) = args {
                children.extend(arg_ids.iter().copied());
            }
        }
        TypedExprKind::TagCall { args, .. } => {
            if let Some(arg_ids) = args {
                children.extend(arg_ids.iter().copied());
            }
        }
        TypedExprKind::When(when_expr) => {
            if let Some(subject_id) = when_expr.subject {
                children.push(subject_id);
            }
            for arm in &when_expr.arms {
                match arm {
                    TypedWhenArm::Cond {
                        condition, body, ..
                    } => {
                        children.push(*condition);
                        children.push(*body);
                    }
                    TypedWhenArm::Is { body, .. } => {
                        children.push(*body);
                    }
                    TypedWhenArm::Else(body, _) => {
                        children.push(*body);
                    }
                }
            }
        }
        TypedExprKind::If(if_expr) => {
            children.push(if_expr.subject);
            children.extend(if_expr.stmts.iter().copied());
            if let Some(ret_id) = if_expr.ret {
                children.push(ret_id);
            }
        }
        TypedExprKind::Loop(typed_loop) => {
            match typed_loop.kind {
                TypedLoopKind::While { condition } => children.push(condition),
                TypedLoopKind::ForIn {
                    variable: _,
                    iterable,
                } => {
                    children.push(iterable);
                }
            }
            children.extend(typed_loop.stmts.iter().copied());
        }
        TypedExprKind::TakePtr(inner)
        | TypedExprKind::Ref(inner)
        | TypedExprKind::Eat(inner)
        | TypedExprKind::ConsumeArg(inner)
        | TypedExprKind::Deref(inner)
        | TypedExprKind::Negate(inner) => {
            children.push(*inner);
        }
        TypedExprKind::TupleGet { base, .. } => {
            children.push(*base);
        }
        TypedExprKind::Destructure { value, .. } => {
            children.push(*value);
        }
        TypedExprKind::RecordSet { base, value, .. }
        | TypedExprKind::TupleSet { base, value, .. } => {
            children.push(*base);
            children.push(*value);
        }
        TypedExprKind::Range { start, end } => {
            children.push(*start);
            children.push(*end);
        }
        TypedExprKind::TupleAlloc { init, .. } => {
            children.push(*init);
        }
        TypedExprKind::TupleLit(items) | TypedExprKind::List(items) => {
            children.extend(items.iter().copied());
        }
        TypedExprKind::BufGet { buf, index } => {
            children.push(*buf);
            children.push(*index);
        }
        TypedExprKind::BufSet { buf, index, value } => {
            children.push(*buf);
            children.push(*index);
            children.push(*value);
        }
        TypedExprKind::Cast { expr: inner, .. } => {
            children.push(*inner);
        }
        TypedExprKind::FormatString(fs) => {
            for part in &fs.parts {
                if let ast::FormatPart::Expr(_, _) = part {
                    // Children not accessible directly from untyped FormatPart
                }
            }
        }
        TypedExprKind::Asm(_asm_expr) => {
            // TODO: extract children from asm operand expressions
        }
        TypedExprKind::Bind {
            stmts,
            body,
            unassigned,
            ..
        } => {
            children.extend(stmts.iter().copied());
            if !unassigned {
                children.push(*body);
            }
        }
        TypedExprKind::Reassign { value, .. } => {
            children.push(*value);
        }
        TypedExprKind::SelfRef { .. } => {}
    }
    children
}

pub(crate) fn eval_const_from_expr(typed: &TypedFileAst, idx: usize) -> Option<ConstValue> {
    if idx >= typed.exprs.kind.len() {
        return None;
    }
    match &typed.exprs.kind[idx] {
        TypedExprKind::Lit(lit) => match lit {
            Literal::Int(n) => Some(ConstValue::Int(*n as i128)),
            Literal::Number(n) => Some(ConstValue::Int(*n as i128)),
            Literal::Float(HashFloat(f)) => Some(ConstValue::Float(HashFloat(*f))),
            Literal::String(s) => Some(ConstValue::String(s.clone())),
        },
        TypedExprKind::TagCall { args, .. } if args.as_ref().is_some_and(|a| a.is_empty()) => {
            // Bare tag call with no args - could be a type constant
            let ty = &typed.exprs.ty[idx];
            Some(reflect_ty_to_const_value(ty))
        }
        _ => typed.exprs.const_value[idx].clone(),
    }
}

fn expr_const_expr(
    kinds: &[TypedExprKind],
    const_values: &[Option<ConstValue>],
    lcv: &HashMap<Intern<String>, ConstValue>,
    expr_id: ExprId,
) -> Option<ConstExpr> {
    let idx = expr_id.as_usize();
    if let Some(cv) = const_values.get(idx).and_then(Clone::clone) {
        return Some(ConstExpr::Value(cv));
    }
    match kinds.get(idx)? {
        TypedExprKind::Lit(lit) => match lit {
            Literal::Int(n) => Some(ConstExpr::Value(ConstValue::Int(*n as i128))),
            Literal::Number(n) => Some(ConstExpr::Value(ConstValue::Int(*n as i128))),
            _ => None,
        },
        TypedExprKind::FnCall { target, args, .. } if args.is_none() => lcv
            .get(&target.0)
            .cloned()
            .map(ConstExpr::Value)
            .or(Some(ConstExpr::Var(target.0))),
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
) -> Vec<(ConstExpr, ConstExpr, bool)> {
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
    let to_expr = |eid: ExprId| -> Option<ConstExpr> {
        let eidx = eid.as_usize();
        match kinds.get(eidx)? {
            TypedExprKind::Lit(lit) => match lit {
                Literal::Int(n) => Some(ConstExpr::Value(ConstValue::Int(*n as i128))),
                Literal::Number(n) => Some(ConstExpr::Value(ConstValue::Int(*n as i128))),
                _ => None,
            },
            TypedExprKind::FnCall { target, args, .. } if args.is_none() => {
                let var = target.0;
                if let Some(cv) = lcv.get(&var) {
                    Some(ConstExpr::Value(cv.clone()))
                } else {
                    Some(ConstExpr::Var(var))
                }
            }
            _ => None,
        }
    };
    let Some(lhs) = to_expr(arg_ids[lhs_idx]) else {
        return Vec::new();
    };
    let Some(rhs) = to_expr(arg_ids[rhs_idx]) else {
        return Vec::new();
    };
    vec![(lhs, rhs, is_lt)]
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
        tracker.register(reference, Some(field.clone()));

        tracker.replace(&field, &ConstraintEnv::default());

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
        tracker.register(reference, Some(field));
        let mut flow = FlowContext::new();

        tracker.permanently_invalidate(&source, &mut flow);

        assert_eq!(
            tracker.validity[&reference],
            ReferenceValidity::PermanentlyInvalid
        );
        assert_eq!(flow.get_var_state(&reference), Some(VarState::Invalidated));
    }
}
