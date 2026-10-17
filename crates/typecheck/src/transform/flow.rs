//! Stage 3: Flow — Compute flow contexts, attach flow flaws.
//!
//! This stage walks the typed expression arena depth-first, computes
//! `FlowContext` at each program point, and attaches flow-related flaws
//! (ownership, bounds, narrowing, lin-value checks, etc.).

use super::callable_signatures::CallableSignatureScope;
use diagnostic::code::{
    BindingCode, OwnershipCode, PatternCode, ProofCode, ReferenceCode, TargetGroupCode, TypeCode,
};
use std::collections::{HashMap, HashSet};

use crate::analysis::{FlowContext, TyOwnershipExt, VarState, expr_is_rematerializable};
use crate::solver::{ConstraintEnv, Predicate, ProveResult, predicate_expr_to_predicate};
use ast::expr::Literal;
use ast::normal_expr::NormalExpr;
use ast::{ConstValue, ParamConvention};
use diagnostic::{Diagnostic, DiagnosticCode, RelatedSpan};

use crate::ty::{Ty, reference_pointee_for_type, reference_semantics_for_type};
use crate::typed::{
    DefId, ExprId, GroupId, Overlap, PlaceId, ReferenceLoan, ReferenceTargetGroup,
    ReferenceTargetSet, TagId, TypedCallableSignature, TypedExprKind, TypedFileAst, TypedLoopKind,
    TypedWhenArm, VariantId,
};
use i256::I256;
use internment::Intern;

mod comparison_values;
mod completions;
mod conditional_bounds;
mod dependent_arrays;
pub(super) use dependent_arrays::field_has_array_dependency;
mod index_bounds;
mod integers;
mod local_callables;
mod loop_targets;
mod loops;
mod ownership;
use local_callables::{LocalOwnershipScope, LocalOwnershipSummary};
use ownership::ownership_state;
mod payload_postconditions;
mod postconditions;
mod rebind_validity;
mod record_constraints;
mod refinement_diagnostics;
mod scalar_state;
mod sequence;
mod terminal;

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
    targets: HashMap<TrackedReference, ReferenceTargetSet>,
    validity: HashMap<TrackedReference, ReferenceValidity>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum TrackedReference {
    Place(PlaceId),
    Loan(ReferenceLoan),
}

impl RefTracker {
    fn new() -> Self {
        Self {
            targets: HashMap::new(),
            validity: HashMap::new(),
        }
    }

    fn register(&mut self, reference: PlaceId, target: Option<ReferenceTargetSet>) {
        if let Some(target) = target {
            let reference = TrackedReference::Place(reference);
            self.targets.insert(reference.clone(), target);
            self.validity.insert(reference, ReferenceValidity::Valid);
        }
    }

    fn replace(&mut self, place: &ReferenceTargetGroup, flow_ctx: &mut FlowContext) {
        self.invalidate_replaced(&ReferenceTargetSet::singleton(place.clone()), flow_ctx);
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
                    .insert(reference.clone(), ReferenceValidity::PermanentlyInvalid);
                if let TrackedReference::Place(place) = reference {
                    flow_ctx.set_place_state(*place, VarState::Invalidated);
                }
            }
        }
    }

    fn merge_branch_validity(&mut self, branches: &[Self]) {
        for branch in branches {
            for (reference, targets) in &branch.targets {
                if matches!(reference, TrackedReference::Loan(_)) {
                    self.targets
                        .entry(reference.clone())
                        .or_default()
                        .extend(targets);
                    self.validity
                        .entry(reference.clone())
                        .or_insert(ReferenceValidity::Valid);
                }
            }
        }
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
        self.invalidate_overlapping(sources, flow_ctx, false);
    }

    fn invalidate_replaced(&mut self, sources: &ReferenceTargetSet, flow_ctx: &mut FlowContext) {
        self.invalidate_overlapping(sources, flow_ctx, true);
    }

    fn invalidate_overlapping(
        &mut self,
        sources: &ReferenceTargetSet,
        flow_ctx: &mut FlowContext,
        preserve_array_views: bool,
    ) {
        for (reference, targets) in &self.targets {
            if targets.iter().any(|target| {
                sources.iter().any(|source| {
                    if preserve_array_views
                        && source.is_within_items_of(target, &flow_ctx.constraint_env)
                    {
                        return false;
                    }
                    target.overlap(source, &flow_ctx.constraint_env) != Overlap::Disjoint
                })
            }) {
                self.validity
                    .insert(reference.clone(), ReferenceValidity::PermanentlyInvalid);
                if let TrackedReference::Place(place) = reference {
                    flow_ctx.set_place_state(*place, VarState::Invalidated);
                }
            }
        }
    }
}

/// Walks the expression arena, computes flow contexts at each program point,
/// and attaches flow-related flaws to expressions.
pub fn stage_flow(
    typed: &mut TypedFileAst,
    cross_file_signatures: &HashMap<DefId, TypedCallableSignature>,
) {
    let _profile = compiler_profile::span(compiler_profile::Phase::Flow);
    typed.sequence_index_checks.clear();
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
                crate::typed::BindBody::Unlowered => return None,
            };
            Some((def_id, ids))
        })
        .collect();

    let local_var_types = collect_local_var_types(typed);
    let callable_signatures = CallableSignatureScope::new(
        cross_file_signatures,
        typed
            .defs
            .iter()
            .map(|(def_id, bind)| (*def_id, TypedCallableSignature::from(bind))),
    );

    payload_postconditions::validate_contracts(typed, &callable_signatures);

    // Track constants detected at compile time.
    let mut const_bindings: HashMap<Intern<String>, bool> = HashMap::new();

    // Process each bind body.
    for (def_id, root_ids) in &bind_bodies {
        let owned_params: Vec<(Intern<String>, Ty, PlaceId, ParamConvention)> = typed
            .defs
            .get(def_id)
            .map(|bind| {
                bind.params
                    .iter()
                    .zip(&bind.param_conventions)
                    .enumerate()
                    .filter_map(|(slot, ((name, ty), convention))| {
                        if !matches!(convention, ParamConvention::Own | ParamConvention::Terminal)
                            || is_reference_type(typed, ty)
                        {
                            return None;
                        }
                        let binder = ast::BinderId::new(
                            typed.file_id.0,
                            ast::BinderOwner::Parameter {
                                definition: *def_id,
                                slot: slot as u32,
                            },
                        );
                        typed
                            .places
                            .iter()
                            .position(|place| place.binder == binder)
                            .map(|place| (*name, ty.clone(), PlaceId(place as u32), *convention))
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
                crate::typed::BindBody::Body { ret: None, .. }
                | crate::typed::BindBody::Unlowered => {
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
                    .zip(bind.param_refinements.iter())
                    .filter_map(|((name, _), refinement)| {
                        refinement
                            .as_ref()
                            .map(|refinement| (*name, refinement.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut flow_ctx = FlowContext::new();
        if let Some(bind) = typed.defs.get(def_id) {
            for (slot, (name, ty)) in bind.params.iter().enumerate() {
                if super::symbolic_values::scalar_type(typed, ty.without_reference_wrappers()) {
                    flow_ctx.scalar_bindings.insert(*name);
                }
                let target = bind
                    .param_groups
                    .get(slot)
                    .copied()
                    .flatten()
                    .map(ReferenceTargetGroup::Param)
                    .or_else(|| {
                        let binder = ast::BinderId::new(
                            typed.file_id.0,
                            ast::BinderOwner::Parameter {
                                definition: *def_id,
                                slot: slot as u32,
                            },
                        );
                        typed
                            .places
                            .iter()
                            .position(|place| place.binder == binder)
                            .map(|place| ReferenceTargetGroup::Local(PlaceId(place as u32)))
                    });
                if let Some(target) = target {
                    flow_ctx
                        .record_targets
                        .insert(*name, ReferenceTargetSet::singleton(target));
                }
            }
        }
        let parameter_types: HashMap<_, _> = typed
            .defs
            .get(def_id)
            .map(|bind| bind.params.iter().cloned().collect())
            .unwrap_or_default();
        let mut parameter_values: HashMap<_, _> = parameter_types
            .keys()
            .map(|name| (*name, NormalExpr::Var(*name)))
            .collect();
        for (name, ty) in &parameter_types {
            flow_ctx.binding_types.insert(*name, ty.clone());
            if let Some(comparisons) =
                comparison_values::proposition_comparisons(ty, &parameter_values)
            {
                flow_ctx.binding_comparisons.insert(*name, comparisons);
            }
            record_constraints::assume_integer_bounds(
                typed,
                ty,
                NormalExpr::Var(*name),
                &mut flow_ctx.constraint_env,
            );
            record_constraints::assume_record_constraints(
                typed,
                &callable_signatures,
                *name,
                ty,
                &mut flow_ctx.constraint_env,
            );
        }
        for (name, refinement) in parameter_constraints {
            super::refinement_terms::extend_values(
                typed,
                &callable_signatures,
                &parameter_types,
                &mut parameter_values,
                &refinement,
            );
            if !super::refinement_terms::values_complete(&refinement, &parameter_values) {
                continue;
            }
            let predicate =
                predicate_expr_to_predicate(&refinement, NormalExpr::Var(name), &parameter_values);
            let predicate = resolve_target_queries_in_predicate(
                &predicate,
                &crate::subst::DepSubst::new(),
                typed,
            );
            flow_ctx.constraint_env.assume(&predicate);
        }
        if let Some((expression, check)) = typed.defs.get(def_id).and_then(|bind| {
            index_bounds::parameter_index_check(typed, bind, &flow_ctx.constraint_env)
        }) {
            typed.sequence_index_checks.insert(expression, check);
        }
        for (_, _, place, _) in &owned_params {
            flow_ctx.set_place_state(*place, VarState::Alive);
        }
        let mut local_const_values: HashMap<Intern<String>, ConstValue> = HashMap::new();
        let mut ref_tracker = RefTracker::new();
        let terminal_receiver = typed.defs.get(def_id).and_then(|bind| {
            let receiver = bind.receiver_type.as_ref()?;
            if !typed.owns_terminal_authority(receiver) {
                return None;
            }
            owned_params
                .iter()
                .find_map(|(name, ty, place, convention)| {
                    (name.as_str() == "self"
                        && ty == receiver
                        && *convention == ParamConvention::Terminal)
                        .then_some(*place)
                })
        });
        let mut ctx = WalkExprCtx {
            owned_parameters: owned_params
                .iter()
                .filter_map(|(_, _, place, convention)| {
                    (*convention == ParamConvention::Own).then_some(*place)
                })
                .collect(),
            local_scope: None,
            local_ownership: HashMap::new(),
            result_exits: Default::default(),
            payload_inputs: Default::default(),
            when_subjects: Vec::new(),
            next_field_snapshot: 0,
            definition: Some(*def_id),
            terminal_receiver,
            projecting: false,
            replacing: None,
            is_constant_definition: typed.defs.get(def_id).is_some_and(|bind| bind.is_constant),
            typed,
            flow_ctx: &mut flow_ctx,
            local_var_types: &local_var_types,
            ref_tracker: &mut ref_tracker,
            const_bindings: &mut const_bindings,
            local_const_values: &mut local_const_values,
            callable_signatures: &callable_signatures,
        };
        ctx.prepare_payload_inputs();
        ctx.result_exits =
            payload_postconditions::result_exits(ctx.typed, &ctx.typed.defs[def_id].body);
        let mut terminated = false;
        for root_id in root_ids.iter() {
            ctx.walk(*root_id);
            if !return_ids.contains(root_id) {
                ctx.validate_discarded_result(*root_id);
            }
            if crate::analysis::expression_returns(ctx.typed.resolved_program(), *root_id) {
                terminated = true;
                break;
            }
        }
        if terminated {
            continue;
        }
        if let Some(proof) = ctx
            .typed
            .defs
            .get(def_id)
            .and_then(|bind| bind.postcondition.clone())
        {
            ctx.verify_postcondition(&proof, None);
        }
        for return_id in return_ids {
            mark_consumed_if_local_ref(ctx.typed, return_id, ctx.flow_ctx);
        }
        emit_unassigned_bindings(ctx.typed, ctx.flow_ctx);
        ctx.validate_terminal_exit(None);
    }

    // Process top-level expressions.
    let mut flow_ctx = FlowContext::new();
    let mut local_const_values: HashMap<Intern<String>, ConstValue> = HashMap::new();
    let mut ref_tracker = RefTracker::new();
    let root_ids = typed.root_exprs.clone();
    let mut ctx = WalkExprCtx {
        owned_parameters: Vec::new(),
        local_scope: None,
        local_ownership: HashMap::new(),
        result_exits: Default::default(),
        payload_inputs: Default::default(),
        when_subjects: Vec::new(),
        next_field_snapshot: 0,
        definition: None,
        terminal_receiver: None,
        projecting: false,
        replacing: None,
        is_constant_definition: false,
        typed,
        flow_ctx: &mut flow_ctx,
        local_var_types: &local_var_types,
        ref_tracker: &mut ref_tracker,
        const_bindings: &mut const_bindings,
        local_const_values: &mut local_const_values,
        callable_signatures: &callable_signatures,
    };
    for root_id in root_ids.iter() {
        ctx.walk(*root_id);
        ctx.validate_discarded_result(*root_id);
    }
    ctx.validate_terminal_exit(root_ids.last().copied());
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
            } if typed
                .exprs
                .place_of(ExprId(idx as u32))
                .flatten()
                .is_some_and(|place| {
                    matches!(flow_ctx.place_state(place), Some(VarState::Declared))
                }) =>
            {
                Some((idx, *name))
            }
            _ => None,
        })
        .collect();
    for (idx, name) in unassigned {
        if !typed.exprs.facts.semantic_facts.flaws[idx].iter().any(|f| {
            f.code == DiagnosticCode::Type(TypeCode::Binding(BindingCode::Unassigned))
                && f.arg("name") == Some(name.as_str())
        }) {
            typed.exprs.facts.semantic_facts.flaws[idx].push(
                // type-unassigned-binding
                Diagnostic::new(
                    BindingCode::Unassigned,
                    format!("binding `{}` is declared but never assigned", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            );
        }
    }
}

fn record_field_index(typed: &TypedFileAst, base: ExprId, field: Intern<String>) -> Option<usize> {
    let ty = typed.exprs.ty_of(base)?;
    let ty =
        reference_pointee_for_type(ty, Some(&typed.type_registry)).unwrap_or_else(|| ty.clone());
    let Ty::Record { fields, .. } = typed.type_registry.resolved_definition_for_type(&ty) else {
        return None;
    };
    fields.iter().position(|(name, _)| *name == field)
}

struct CallRefinementInputs<'a> {
    args: &'a [ExprId],
    argument_values: &'a [Option<NormalExpr>],
    signature: &'a TypedCallableSignature,
    substituted_return: Option<&'a Ty>,
}

fn validate_call_refinements(
    typed: &TypedFileAst,
    call: CallRefinementInputs<'_>,
    constraints: &ConstraintEnv,
    signatures: &CallableSignatureScope<'_>,
    local_values: &HashMap<Intern<String>, ConstValue>,
) -> Vec<Diagnostic> {
    let CallRefinementInputs {
        args,
        argument_values,
        signature,
        substituted_return,
    } = call;
    let mut substitutions = HashMap::new();
    let mut argument_types = HashMap::new();
    let mut type_substitutions = HashMap::new();
    if let Some(substituted_return) = substituted_return {
        collect_dependent_type_substitutions(
            &signature.return_type,
            substituted_return,
            &mut type_substitutions,
        );
    }
    for (slot, ((name, formal_ty), arg)) in signature.params.iter().zip(args).enumerate() {
        if let Some(actual) = argument_values.get(slot).cloned().flatten().or_else(|| {
            super::symbolic_values::normal_expr(typed, Some(signatures), local_values, *arg)
        }) {
            substitutions.insert(*name, actual);
        }
        if let Some(actual_ty) = typed.exprs.ty_of(*arg) {
            argument_types.insert(*name, actual_ty.clone());
            collect_dependent_type_substitutions(formal_ty, actual_ty, &mut type_substitutions);
            collect_dependent_substitutions(
                formal_ty,
                actual_ty,
                &mut substitutions,
                &typed.type_registry,
            );
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
        let argument_knowledge = typed
            .exprs
            .integer_knowledge_of(*arg)
            .and_then(Option::as_ref);
        let argument_domain = argument_knowledge
            .and_then(super::place::knowledge_domain)
            .or_else(|| {
                if matches!(
                    argument_knowledge,
                    Some(ast::integer::IntegerKnowledge::Poison)
                ) {
                    return None;
                }
                let ty = typed.exprs.ty_of(*arg)?.without_reference_wrappers();
                let validity = typed.type_registry.integer_validity_for_type(ty)?;
                let (lower, upper) = validity.domain().closed_bounds()?;
                if !matches!(lower, NormalExpr::Value(ConstValue::Int(_)))
                    || !matches!(upper, NormalExpr::Value(ConstValue::Int(_)))
                {
                    return None;
                }
                Some(validity.domain().clone())
            });
        let expected_domain = ast::integer::IntegerDomain::from_named_refinement(refinement, *name);
        let independent = super::refinement_terms::predicate_names(refinement)
            .iter()
            .all(|dependency| dependency == name);
        if independent
            && argument_domain.as_ref().is_some_and(|actual| {
                actual.prove_containment(&expected_domain) == ast::integer::ContainmentProof::Proven
            })
        {
            continue;
        }
        super::refinement_terms::extend_values(
            typed,
            signatures,
            &argument_types,
            &mut substitutions,
            refinement,
        );
        let actual = substitutions
            .get(name)
            .cloned()
            .filter(|_| super::refinement_terms::values_complete(refinement, &substitutions));
        let Some(actual) = actual else {
            flaws.push(refinement_diagnostics::argument(name.as_str(), refinement));
            continue;
        };
        let predicate = predicate_expr_to_predicate(refinement, actual.clone(), &substitutions);
        let proven_by_target = typed
            .target_layout
            .is_some_and(|layout| target_alignment_guarantees(&predicate, &actual, *name, layout));
        let substitution = crate::subst::DepSubst {
            types: type_substitutions.clone(),
            consts: substitutions.clone(),
        };
        let resolved_predicate =
            resolve_target_queries_in_predicate(&predicate, &substitution, typed);
        let mut argument_constraints = constraints.clone();
        if typed.target_layout.is_none() {
            for dependency in super::refinement_terms::predicate_names(refinement) {
                if substitutions.get(&dependency) == Some(&NormalExpr::Var(dependency))
                    && let Some((low, high)) =
                        super::refinement_terms::constant_bounds(typed, dependency)
                {
                    argument_constraints
                        .assume(&Predicate::Ge(NormalExpr::Var(dependency), low.into()));
                    argument_constraints
                        .assume(&Predicate::Le(NormalExpr::Var(dependency), high.into()));
                }
            }
        }
        if let Some(hull) = argument_domain
            .as_ref()
            .and_then(|domain| domain.storage_hull())
        {
            argument_constraints.assume(&Predicate::Ge(actual.clone(), hull.min().into()));
            argument_constraints.assume(&Predicate::Le(actual.clone(), hull.max().into()));
        }
        match argument_constraints.prove(&resolved_predicate, &HashMap::new()) {
            ProveResult::Proven => {}
            ProveResult::Disproven => flaws.push(
                // type-parameter-refinement-failed
                Diagnostic::new(
                    ProofCode::ParameterRefinementFailed,
                    format!(
                        "argument `{}` does not satisfy its refinement",
                        name.as_str()
                    ),
                )
                .with_arg("name", name.as_str().to_string()),
            ),
            ProveResult::Unknown if proven_by_target => {}
            ProveResult::Unknown => {
                flaws.push(refinement_diagnostics::argument(name.as_str(), refinement));
            }
        }
    }
    flaws
}

fn resolve_target_queries_in_predicate(
    predicate: &crate::solver::Predicate,
    substitution: &crate::subst::DepSubst,
    typed: &TypedFileAst,
) -> crate::solver::Predicate {
    use crate::solver::Predicate;

    let resolve = |expr: &NormalExpr| resolve_target_queries_in_normal(expr, substitution, typed);
    match predicate {
        Predicate::Lt(left, right) => Predicate::Lt(resolve(left), resolve(right)),
        Predicate::Gt(left, right) => Predicate::Gt(resolve(left), resolve(right)),
        Predicate::Le(left, right) => Predicate::Le(resolve(left), resolve(right)),
        Predicate::Ge(left, right) => Predicate::Ge(resolve(left), resolve(right)),
        Predicate::Eq(left, right) => Predicate::Eq(resolve(left), resolve(right)),
        Predicate::Ne(left, right) => Predicate::Ne(resolve(left), resolve(right)),
        Predicate::Divisible(value, divisor) => {
            layout_divisibility(resolve(value), resolve(divisor), typed)
        }
        Predicate::And(predicates) => Predicate::And(
            predicates
                .iter()
                .map(|predicate| {
                    resolve_target_queries_in_predicate(predicate, substitution, typed)
                })
                .collect(),
        ),
        Predicate::Opaque(proposition) => Predicate::Opaque(proposition.clone()),
    }
}

fn resolve_target_queries_in_normal(
    expr: &NormalExpr,
    substitution: &crate::subst::DepSubst,
    typed: &TypedFileAst,
) -> NormalExpr {
    match expr {
        NormalExpr::Add(left, right) => NormalExpr::Add(
            Box::new(resolve_target_queries_in_normal(left, substitution, typed)),
            Box::new(resolve_target_queries_in_normal(right, substitution, typed)),
        ),
        NormalExpr::Sub(left, right) => NormalExpr::Sub(
            Box::new(resolve_target_queries_in_normal(left, substitution, typed)),
            Box::new(resolve_target_queries_in_normal(right, substitution, typed)),
        ),
        NormalExpr::Mul(left, right) => NormalExpr::Mul(
            Box::new(resolve_target_queries_in_normal(left, substitution, typed)),
            Box::new(resolve_target_queries_in_normal(right, substitution, typed)),
        ),
        NormalExpr::TargetQuery { .. } => {
            let substituted = substitution.apply_to_normal(expr);
            let NormalExpr::TargetQuery { kind, operand } = &substituted else {
                unreachable!()
            };
            query_operand_type(typed, operand)
                .and_then(|ty| match typed.target_layout {
                    Some(layout) => layout.query(*kind, &ty, Some(&typed.type_registry)).ok(),
                    None => invariant_layout_query(*kind, &ty, &typed.type_registry),
                })
                .map(|value| NormalExpr::Value(ConstValue::Int(value.into())))
                .unwrap_or(substituted)
        }
        NormalExpr::Value(_) | NormalExpr::Var(_) | NormalExpr::Inferred(_) => expr.clone(),
    }
}

fn query_operand_type(typed: &TypedFileAst, operand: &ast::TypeReference) -> Option<Ty> {
    let mut types = HashMap::new();
    let mut pending = vec![operand];
    for _ in 0..128 {
        let Some(reference) = pending.pop() else {
            return Some(super::lower_ty::resolve_type_reference(
                operand,
                &types,
                &HashMap::new(),
                None,
                &typed.type_registry,
            ));
        };
        match reference {
            ast::TypeReference::Unit => {}
            ast::TypeReference::Pointer(inner) => pending.push(inner),
            ast::TypeReference::Nominal(name) => {
                types.insert(*name, typed.tag_types.get(&TagId(*name))?.clone());
            }
            ast::TypeReference::Application { name, arguments } => {
                types.insert(*name, typed.tag_types.get(&TagId(*name))?.clone());
                pending.extend(arguments);
            }
        }
    }
    None
}

fn layout_divisibility(value: NormalExpr, divisor: NormalExpr, typed: &TypedFileAst) -> Predicate {
    use crate::layout::{LayoutTarget, TargetLayout};
    let fallback = Predicate::Divisible(value.clone(), divisor.clone());
    if typed.target_layout.is_some() {
        return fallback;
    }
    let NormalExpr::TargetQuery {
        kind: ast::TargetQueryKind::Alignment,
        operand,
    } = &divisor
    else {
        return fallback;
    };
    let Some(ty) = query_operand_type(typed, operand) else {
        return fallback;
    };
    let mut proofs = Vec::new();
    for target in [
        LayoutTarget::X86_64,
        LayoutTarget::Arm64,
        LayoutTarget::Wasm32,
    ] {
        let Ok(alignment) = TargetLayout::new(target).query(
            ast::TargetQueryKind::Alignment,
            &ty,
            Some(&typed.type_registry),
        ) else {
            return fallback;
        };
        proofs.push(Predicate::Divisible(
            value.clone(),
            NormalExpr::from(alignment as i128),
        ));
    }
    Predicate::And(proofs)
}

pub(super) fn invariant_layout_query(
    kind: ast::TargetQueryKind,
    ty: &Ty,
    registry: &crate::TypeRegistry,
) -> Option<u64> {
    use crate::layout::{LayoutTarget, TargetLayout};
    let mut value = None;
    for target in [
        LayoutTarget::X86_64,
        LayoutTarget::Arm64,
        LayoutTarget::Wasm32,
    ] {
        let current = TargetLayout::new(target)
            .query(kind, ty, Some(registry))
            .ok()?;
        if value.is_some_and(|previous| previous != current) {
            return None;
        }
        value = Some(current);
    }
    value
}

pub(super) fn layout_query_bounds(kind: ast::TargetQueryKind) -> (u64, u64) {
    use crate::layout::{LayoutTarget, TargetLayout};
    match kind {
        ast::TargetQueryKind::Size => (0, u64::MAX),
        ast::TargetQueryKind::Alignment => (1, u64::MAX),
        ast::TargetQueryKind::IndexBits => {
            let widths = [
                LayoutTarget::X86_64,
                LayoutTarget::Arm64,
                LayoutTarget::Wasm32,
            ]
            .map(|target| u64::from(TargetLayout::new(target).index_bits()));
            (*widths.iter().min().unwrap(), *widths.iter().max().unwrap())
        }
    }
}

fn collect_dependent_type_substitutions(
    formal: &Ty,
    actual: &Ty,
    substitutions: &mut HashMap<Intern<String>, Ty>,
) {
    match (formal, actual) {
        (Ty::Opaque(name), actual) => {
            substitutions.insert(*name, actual.clone());
        }
        (
            Ty::Named {
                instance: formal, ..
            },
            Ty::Named {
                instance: actual, ..
            },
        ) if formal.declaration == actual.declaration => {
            for ((_, formal), (_, actual)) in formal.arguments.iter().zip(&actual.arguments) {
                if let (ast::TyArg::Type(formal), ast::TyArg::Type(actual)) = (formal, actual) {
                    collect_dependent_type_substitutions(formal, actual, substitutions);
                }
            }
        }
        (
            Ty::Ref { inner: formal, .. } | Ty::Ptr { inner: formal },
            Ty::Ref { inner: actual, .. } | Ty::Ptr { inner: actual },
        ) => collect_dependent_type_substitutions(formal, actual, substitutions),
        _ => {}
    }
}

fn target_alignment_guarantees(
    predicate: &crate::solver::Predicate,
    actual: &NormalExpr,
    parameter: Intern<String>,
    layout: crate::layout::TargetLayout,
) -> bool {
    use crate::solver::Predicate;

    let integer = |expr: &NormalExpr| match expr {
        NormalExpr::Value(ConstValue::Int(value)) => Some(*value),
        _ => None,
    };
    match predicate {
        Predicate::And(predicates) => predicates
            .iter()
            .all(|predicate| target_alignment_guarantees(predicate, actual, parameter, layout)),
        Predicate::Ge(left, right) if left == actual => {
            integer(right).is_some_and(|minimum| minimum <= 1.into())
        }
        Predicate::Le(left, right) if left == actual => {
            integer(right).is_some_and(|maximum| maximum >= layout.max_alignment().into())
        }
        Predicate::Divisible(dividend, divisor) if divisor == actual => {
            integer(dividend).is_some_and(|value| value == layout.max_alignment().into())
        }
        Predicate::Opaque(proposition) => {
            target_alignment_proposition_guaranteed(proposition, parameter, layout)
        }
        _ => false,
    }
}

fn target_alignment_proposition_guaranteed(
    proposition: &ast::ProofProposition,
    parameter: Intern<String>,
    layout: crate::layout::TargetLayout,
) -> bool {
    match proposition {
        ast::ProofProposition::And(left, right) => {
            target_alignment_proposition_guaranteed(left, parameter, layout)
                && target_alignment_proposition_guaranteed(right, parameter, layout)
        }
        ast::ProofProposition::InRange { value, start, end } => {
            let ast::ProofTerm::Name(value) = value else {
                return false;
            };
            let ast::ProofTerm::Value(start) = start else {
                return false;
            };
            let ast::ProofTerm::Value(end) = end else {
                return false;
            };
            (*value == parameter || value.as_str() == "self")
                && *start <= 1.into()
                && *end >= layout.max_alignment().into()
        }
        ast::ProofProposition::Compare {
            left: ast::ProofTerm::Remainder(dividend, divisor),
            relation: ast::ProofRelation::Equal,
            right,
        } => {
            let ast::ProofTerm::Value(dividend) = dividend.as_ref() else {
                return false;
            };
            let ast::ProofTerm::Name(divisor) = divisor.as_ref() else {
                return false;
            };
            let ast::ProofTerm::Value(remainder) = right else {
                return false;
            };
            (*divisor == parameter || divisor.as_str() == "self")
                && *remainder == 0.into()
                && *dividend == layout.max_alignment().into()
        }
        _ => false,
    }
}

fn collect_dependent_substitutions(
    formal: &Ty,
    actual: &Ty,
    substitutions: &mut HashMap<Intern<String>, NormalExpr>,
    type_registry: &crate::TypeRegistry,
) {
    match (
        reference_instance(formal, type_registry),
        reference_instance(actual, type_registry),
    ) {
        (Some((formal_inner, _)), None) => {
            collect_dependent_substitutions(&formal_inner, actual, substitutions, type_registry)
        }
        (None, Some((actual_inner, _))) => {
            collect_dependent_substitutions(formal, &actual_inner, substitutions, type_registry)
        }
        (Some((formal_inner, _)), Some((actual_inner, _))) => collect_dependent_substitutions(
            &formal_inner,
            &actual_inner,
            substitutions,
            type_registry,
        ),
        (None, None) => {
            if let (
                Ty::Ref {
                    inner: formal_ref, ..
                },
                Ty::Ref {
                    inner: actual_ref, ..
                },
            ) = (formal, actual)
            {
                collect_dependent_substitutions(
                    formal_ref,
                    actual_ref,
                    substitutions,
                    type_registry,
                );
                return;
            }
            let formal = match formal {
                Ty::Array {
                    elem: formal_elem,
                    size: formal_size,
                } => (formal_elem, formal_size),
                _ => return,
            };
            let (formal_elem, formal_size) = formal;
            let actual_size = match actual {
                Ty::Array { elem: _, size } => size,
                _ => return,
            };
            let Ty::Array {
                elem: actual_elem, ..
            } = actual
            else {
                return;
            };
            if let NormalExpr::Var(name) = formal_size {
                substitutions.insert(*name, actual_size.clone());
            }
            collect_dependent_substitutions(formal_elem, actual_elem, substitutions, type_registry);
        }
    }
}

fn reference_instance(ty: &Ty, type_registry: &crate::TypeRegistry) -> Option<(Ty, bool)> {
    type_registry
        .reference_former_for_type(ty)
        .map(|reference| {
            (
                reference.pointee,
                matches!(
                    reference.permission,
                    crate::type_registry::ReferencePermission::Mutate
                ),
            )
        })
}

fn mark_consumed_if_local_ref(
    typed: &mut TypedFileAst,
    expr_id: ExprId,
    flow_ctx: &mut FlowContext,
) {
    if expr_is_rematerializable(typed, expr_id) {
        return;
    }
    if let Some(ty) = typed.exprs.ty_of(expr_id)
        && typed.type_registry.owns_storage_value(ty)
        && typed.call_argument_is_shared(expr_id)
    {
        let span = typed.exprs.span[expr_id.index()];
        typed.exprs.facts.semantic_facts.flaws[expr_id.index()].push(
            // type-storage-authority
            Diagnostic::new(
                OwnershipCode::StorageAuthority,
                "owned storage cannot be moved out of a shared reference",
            )
            .at_span_id(span, &typed.span_table),
        );
        return;
    }
    if typed.borrows_value(expr_id)
        && typed
            .exprs
            .ty_of(expr_id)
            .is_some_and(|ty| typed.borrowed_value_needs_ownership(ty))
    {
        let span = typed.exprs.span[expr_id.index()];
        typed.exprs.facts.semantic_facts.flaws[expr_id.index()].push(
            // type-move-out-of-borrow
            Diagnostic::new(
                OwnershipCode::MoveOutOfBorrow,
                "owned values cannot be consumed through a borrow",
            )
            .at_span_id(span, &typed.span_table),
        );
        return;
    }
    let Some(place) = typed.exprs.place_of(expr_id).flatten() else {
        return;
    };
    flow_ctx.set_place_state(place, VarState::Consumed);
}

fn is_explicit_rebind_consumption(
    typed: &TypedFileAst,
    expr_id: ExprId,
    target_name: Intern<String>,
) -> bool {
    let Some(kind) = typed.exprs.kind_of(expr_id) else {
        return false;
    };
    match kind {
        TypedExprKind::Ref(inner)
        | TypedExprKind::TakePtr(inner)
        | TypedExprKind::Deref(inner)
        | TypedExprKind::Negate(inner)
        | TypedExprKind::Cast { expr: inner, .. }
        | TypedExprKind::UnionPayload { expr: inner, .. } => {
            expr_consumes_target(typed, *inner, target_name)
        }
        TypedExprKind::AggregateGet { base, .. } | TypedExprKind::BufGet { buf: base, .. } => {
            expr_consumes_target(typed, *base, target_name)
        }
        TypedExprKind::FnCall {
            args: Some(args), ..
        } => {
            for arg in args {
                if expr_consumes_target(typed, *arg, target_name) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn expr_consumes_target(
    typed: &TypedFileAst,
    expr_id: ExprId,
    target_name: Intern<String>,
) -> bool {
    let Some(kind) = typed.exprs.kind_of(expr_id) else {
        return false;
    };
    match kind {
        TypedExprKind::Ref(inner)
        | TypedExprKind::TakePtr(inner)
        | TypedExprKind::AggregateGet { base: inner, .. }
        | TypedExprKind::BufGet { buf: inner, .. }
        | TypedExprKind::Negate(inner)
        | TypedExprKind::Cast { expr: inner, .. }
        | TypedExprKind::UnionPayload { expr: inner, .. } => {
            expr_consumes_target(typed, *inner, target_name)
        }
        TypedExprKind::FnCall { target, .. } if target.0 == target_name => true,
        _ => false,
    }
}

fn expression_consumes_place(
    typed: &TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
    expression: ExprId,
    place: crate::typed::PlaceId,
) -> bool {
    let Some(kind) = typed.exprs.kind_of(expression) else {
        return false;
    };
    match kind {
        TypedExprKind::FnCall {
            target,
            args: Some(args),
            ..
        } => {
            let Some(signature) = signatures.get(target) else {
                return false;
            };
            args.iter()
                .zip(&signature.param_conventions)
                .any(|(arg, convention)| {
                    (matches!(convention, ParamConvention::Own | ParamConvention::Terminal)
                        && !matches!(
                            typed.exprs.kind_of(*arg),
                            Some(TypedExprKind::Ref(_) | TypedExprKind::TakePtr(_))
                        )
                        && typed.exprs.place_of(*arg).flatten() == Some(place))
                        || expression_consumes_place(typed, signatures, *arg, place)
                })
        }
        TypedExprKind::Cast { expr, .. } | TypedExprKind::Negate(expr) => {
            expression_consumes_place(typed, signatures, *expr, place)
        }
        _ => false,
    }
}

fn is_reference_type(typed: &TypedFileAst, ty: &Ty) -> bool {
    reference_semantics_for_type(ty, Some(&typed.type_registry)).is_some()
}

struct WalkExprCtx<'a> {
    owned_parameters: Vec<PlaceId>,
    local_scope: Option<LocalOwnershipScope>,
    local_ownership: HashMap<ExprId, LocalOwnershipSummary>,
    result_exits: HashSet<ExprId>,
    payload_inputs: HashMap<Intern<String>, NormalExpr>,
    next_field_snapshot: u64,
    when_subjects: Vec<ExprId>,
    definition: Option<DefId>,
    terminal_receiver: Option<PlaceId>,
    projecting: bool,
    replacing: Option<PlaceId>,
    is_constant_definition: bool,
    typed: &'a mut TypedFileAst,
    flow_ctx: &'a mut FlowContext,
    local_var_types: &'a HashMap<Intern<String>, Ty>,
    ref_tracker: &'a mut RefTracker,
    const_bindings: &'a mut HashMap<Intern<String>, bool>,

    local_const_values: &'a mut HashMap<Intern<String>, ConstValue>,
    callable_signatures: &'a CallableSignatureScope<'a>,
}

#[derive(Clone)]
struct FlowSnapshot {
    flow_ctx: FlowContext,
    ref_tracker: RefTracker,
    const_bindings: HashMap<Intern<String>, bool>,
    local_const_values: HashMap<Intern<String>, ConstValue>,
}

impl<'a> WalkExprCtx<'a> {
    fn apply_local_call_writes(&mut self, expression: ExprId, callable: ExprId) {
        let writes = super::local_call_effects::written_places(self.typed, callable, |target| {
            self.callable_signatures
                .get(&target)
                .is_none_or(super::local_call_effects::may_write)
        });
        let mut names: Vec<_> = if let Some(writes) = writes {
            let mut targets = ReferenceTargetSet::default();
            for place in writes {
                let info = &self.typed.places[place.0 as usize];
                let target = match info.binder.owner {
                    ast::BinderOwner::Parameter { definition, slot } => self
                        .typed
                        .defs
                        .get(&definition)
                        .and_then(|bind| bind.param_groups.get(slot as usize))
                        .copied()
                        .flatten()
                        .map(ReferenceTargetGroup::Param)
                        .unwrap_or(ReferenceTargetGroup::Local(place)),
                    _ => ReferenceTargetGroup::Local(place),
                };
                targets.extend(&ReferenceTargetSet::singleton(target));
            }
            self.flow_ctx
                .record_targets
                .iter()
                .filter(|(_, known)| !known.is_disjoint(&targets, &self.flow_ctx.constraint_env))
                .map(|(name, _)| *name)
                .collect()
        } else {
            self.flow_ctx.binding_types.keys().copied().collect()
        };
        names.sort_by_key(|name| name.as_str().to_string());
        for name in names {
            self.snapshot_reference_binding(expression, name);
        }
    }

    fn walk_replacement_value(&mut self, target: ExprId, value: ExprId) {
        let previous = self.replacing;
        self.replacing = self.typed.exprs.place_of(target).flatten();
        self.walk(value);
        let targets = self.record_targets_for(target);
        self.forget_union_variant_exclusions(targets.as_ref());
        self.replacing = previous;
    }
    fn snapshot(&self) -> FlowSnapshot {
        FlowSnapshot {
            flow_ctx: self.flow_ctx.clone(),
            ref_tracker: self.ref_tracker.clone(),
            const_bindings: self.const_bindings.clone(),
            local_const_values: self.local_const_values.clone(),
        }
    }

    fn restore(&mut self, snapshot: FlowSnapshot) {
        *self.flow_ctx = snapshot.flow_ctx;
        *self.ref_tracker = snapshot.ref_tracker;
        *self.const_bindings = snapshot.const_bindings;
        *self.local_const_values = snapshot.local_const_values;
    }

    fn validate_projected_write(&mut self, expression: ExprId, value: ExprId) {
        self.validate_record_constraints(expression, value);
        self.remember_field_write(expression, value);
        let index = expression.as_usize();
        let Some(place_id) = self.typed.exprs.place_of(expression).flatten() else {
            return;
        };
        let place = &self.typed.places[place_id.0 as usize];
        if !place.mutable {
            self.typed.exprs.facts.semantic_facts.flaws[index].push(
                // type-rebind-immutable
                Diagnostic::new(
                    BindingCode::RebindImmutable,
                    format!("cannot rebind immutable value `{}`", place.name.as_str()),
                )
                .with_arg("name", place.name.as_str().to_string()),
            );
        }
        let raw_store = match self.typed.exprs.kind_of(expression) {
            Some(TypedExprKind::BufSet { buf, .. }) => {
                self.typed.exprs.ty_of(*buf).is_some_and(|ty| {
                    matches!(
                        self.typed
                            .type_registry
                            .resolved_definition_for_type(ty.without_reference_wrappers()),
                        Ty::Ptr { .. } | Ty::OwnedStorage { .. }
                    )
                })
            }
            _ => false,
        };
        let operator = match self.typed.exprs.kind_of(expression) {
            Some(
                TypedExprKind::RecordSet { operator, .. }
                | TypedExprKind::AggregateSet { operator, .. }
                | TypedExprKind::BufSet { operator, .. },
            ) => operator.as_ref(),
            _ => None,
        };
        let consumes_previous = match operator {
            Some(crate::typed::ResolvedCompoundOperator::Callable { target, .. }) => self
                .callable_signatures
                .get(target)
                .is_some_and(|signature| {
                    matches!(
                        signature.param_conventions.first(),
                        Some(ParamConvention::Own | ParamConvention::Terminal)
                    ) && signature
                        .params
                        .first()
                        .is_some_and(|(_, ty)| !is_reference_type(self.typed, ty))
                }),
            _ => false,
        };
        let payload_free_sum = matches!(
            self.typed.type_registry.resolved_definition_for_type(&place.ty),
            Ty::Union { variants, .. }
                if variants.iter().all(|variant| {
                    variant.kind == ast::ty::UnionVariantKind::Constructor
                        && variant.fields.is_empty()
                })
        );
        let base = match self.typed.exprs.kind_of(expression) {
            Some(
                TypedExprKind::RecordSet { base, .. } | TypedExprKind::AggregateSet { base, .. },
            ) => Some(*base),
            Some(TypedExprKind::BufSet { buf, .. }) => Some(*buf),
            _ => None,
        };
        let temporary = base.is_some_and(|base| self.typed.temporary_owner(base).is_some());
        if !temporary
            && !raw_store
            && !consumes_previous
            && !payload_free_sum
            && !place.ty.is_structurally_discardable()
            && !is_reference_type(self.typed, &place.ty)
            && !expression_consumes_place(self.typed, self.callable_signatures, value, place_id)
        {
            self.typed.exprs.facts.semantic_facts.flaws[index].push(
                // type-rebind-would-discard-value
                Diagnostic::new(
                    BindingCode::RebindWouldDiscardValue,
                    format!(
                        "replacing `{}` requires consuming or transferring its previous value",
                        place.name.as_str()
                    ),
                )
                .with_arg("name", place.name.as_str().to_string()),
            );
        }
    }

    fn apply_condition_ownership_join(
        &mut self,
        paths: &[Vec<ConditionPattern<'_>>],
        move_bindings: bool,
    ) {
        let incoming_flow = self.flow_ctx.clone();
        let incoming_refs = self.ref_tracker.clone();
        let mut branch_flows = Vec::with_capacity(paths.len());
        let mut branch_refs = Vec::with_capacity(paths.len());
        for path in paths {
            *self.flow_ctx = incoming_flow.clone();
            *self.ref_tracker = incoming_refs.clone();
            for fact in path {
                let subject = &fact.subject;
                if fact.stable {
                    self.exclude_pattern_variants(fact.subject, fact.pattern, fact.taken);
                }
                if !move_bindings
                    || !fact.taken
                    || self.typed.borrows_value(*subject)
                    || self.pattern_discards_owned_fields(fact.pattern, *subject)
                    || !pattern_moves_subject(fact.pattern)
                {
                    continue;
                }
                self.validate_unpack_authority(*subject, *subject);
                mark_consumed_if_local_ref(self.typed, *subject, self.flow_ctx);
                if let Some(source) = self
                    .typed
                    .exprs
                    .target_group_of(*subject)
                    .cloned()
                    .flatten()
                {
                    self.ref_tracker
                        .permanently_invalidate(&source, self.flow_ctx);
                }
            }
            branch_flows.push(self.flow_ctx.clone());
            branch_refs.push(self.ref_tracker.clone());
        }
        *self.flow_ctx = incoming_flow;
        self.flow_ctx.merge_branch_states(&branch_flows);
        *self.ref_tracker = incoming_refs;
        self.ref_tracker.merge_branch_validity(&branch_refs);
    }

    fn walk_terminating_condition_branch(
        &mut self,
        branch: &[ExprId],
        ownership_paths: &[Vec<ConditionPattern<'_>>],
        returned: Option<ExprId>,
        site: ExprId,
        owned_bindings: &[PlaceId],
        postcondition: Option<&crate::typed::TypedPostcondition>,
    ) {
        let snapshot = self.snapshot();
        self.apply_condition_ownership_join(ownership_paths, true);
        for place in owned_bindings {
            self.flow_ctx.set_place_state(*place, VarState::Alive);
        }
        for expr in branch {
            self.walk(*expr);
            if Some(*expr) != returned {
                self.validate_discarded_result(*expr);
            }
        }
        if let Some(proof) = postcondition {
            self.verify_postcondition(proof, Some(site));
        }
        if let Some(returned) = returned {
            mark_consumed_if_local_ref(self.typed, returned, self.flow_ctx);
        }
        self.validate_terminal_exit(Some(site));
        self.restore(snapshot);
    }

    fn walk_sequence_iteration(&mut self, statements: &[ExprId]) {
        let constants = self.const_bindings.clone();
        let values = self.local_const_values.clone();
        for statement in statements {
            self.walk(*statement);
            self.validate_discarded_result(*statement);
        }
        for statement in statements {
            if let TypedExprKind::Bind { name, .. } = self.typed.exprs.kind[statement.index()] {
                if let Some(value) = constants.get(&name) {
                    self.const_bindings.insert(name, *value);
                } else {
                    self.const_bindings.remove(&name);
                }
                if let Some(value) = values.get(&name) {
                    self.local_const_values.insert(name, value.clone());
                } else {
                    self.local_const_values.remove(&name);
                }
            }
        }
    }

    fn walk_repeating_body(
        &mut self,
        body: &[ExprId],
        condition: Option<(&crate::typed::TypedCondition, &[PlaceId])>,
    ) {
        let snapshot = self.snapshot();
        if let Some((condition, owned_bindings)) = condition {
            let paths = self.condition_pattern_paths(condition, true);
            self.apply_condition_ownership_join(&paths, true);
            for place in owned_bindings {
                self.flow_ctx.set_place_state(*place, VarState::Alive);
            }
        }
        for expr in body {
            self.walk(*expr);
            self.validate_discarded_result(*expr);
        }
        let first_flow = self.flow_ctx.clone();
        let first_refs = self.ref_tracker.clone();

        let mut repeats = true;
        if let Some((condition, owned_bindings)) = condition {
            let values = self.walk_condition_values(condition);
            let facts = self.condition_facts(condition, true, &values);
            repeats = !facts.iter().any(|(left, right, strict)| {
                self.flow_ctx.constraint_env.prove(
                    &if *strict {
                        Predicate::Lt(left.clone(), right.clone())
                    } else {
                        Predicate::Le(left.clone(), right.clone())
                    },
                    &HashMap::new(),
                ) == ProveResult::Disproven
            });
            for (left, right, strict) in facts {
                if strict {
                    self.flow_ctx.constraint_env.known_lt.push((left, right));
                } else {
                    self.flow_ctx.constraint_env.known_le.push((left, right));
                }
            }
            let paths = self.condition_pattern_paths(condition, true);
            self.apply_condition_ownership_join(&paths, true);
            for place in owned_bindings {
                self.flow_ctx.set_place_state(*place, VarState::Alive);
            }
        }
        if repeats {
            for expr in body {
                self.walk(*expr);
            }
        }

        let incoming_flow = snapshot.flow_ctx.clone();
        let incoming_refs = snapshot.ref_tracker.clone();
        let mut iteration_owners = HashSet::new();
        let iteration_places: Vec<_> = self
            .typed
            .places
            .iter()
            .enumerate()
            .filter_map(|(index, place)| {
                let id = PlaceId(index as u32);
                let local = match place.parent {
                    None => {
                        matches!(
                            place.binder.owner,
                            ast::BinderOwner::Expression(_) | ast::BinderOwner::Pattern { .. }
                        ) && incoming_flow.place_state(id).is_none()
                    }
                    Some(parent) => {
                        self.typed.owns_projection(id) && iteration_owners.contains(&parent)
                    }
                };
                if !local {
                    return None;
                }
                iteration_owners.insert(id);
                first_flow.place_state(id).map(|state| (id, state))
            })
            .collect();
        *self.flow_ctx = incoming_flow.clone();
        self.flow_ctx
            .merge_branch_states(&[incoming_flow, first_flow]);
        for (place, state) in iteration_places {
            self.flow_ctx.set_place_state(place, state);
        }
        *self.ref_tracker = incoming_refs.clone();
        self.ref_tracker
            .merge_branch_validity(&[incoming_refs, first_refs]);
        self.const_bindings.clone_from(&snapshot.const_bindings);
        self.local_const_values
            .clone_from(&snapshot.local_const_values);
    }

    fn apply_when_condition(&mut self, condition: &crate::typed::TypedCondition, taken: bool) {
        let values = self.walk_condition_values(condition);
        for (left, right, strict) in self.condition_facts(condition, taken, &values) {
            if strict {
                self.flow_ctx.constraint_env.known_lt.push((left, right));
            } else {
                self.flow_ctx.constraint_env.known_le.push((left, right));
            }
        }
        let paths = self.condition_pattern_paths(condition, taken);
        self.apply_condition_ownership_join(&paths, taken);
    }

    fn reject_when_arms(&mut self, arms: &[TypedWhenArm], subject: Option<ExprId>) {
        for arm in arms {
            match arm {
                TypedWhenArm::Cond { condition, .. } => self.apply_when_condition(condition, false),
                TypedWhenArm::Is { pattern, .. } => {
                    if let Some(subject) = subject {
                        self.apply_when_condition(
                            &crate::typed::TypedCondition::Is {
                                subject,
                                pattern: pattern.clone(),
                            },
                            false,
                        );
                    }
                }
                TypedWhenArm::Else(..) => {}
            }
        }
    }

    fn enter_when_arm(
        &mut self,
        arm: &TypedWhenArm,
        subject: Option<ExprId>,
        preceding_arms: &[TypedWhenArm],
        site: ExprId,
    ) -> ExprId {
        self.reject_when_arms(preceding_arms, subject);
        match arm {
            TypedWhenArm::Cond {
                condition,
                body,
                owned_bindings,
                ..
            } => {
                self.apply_when_condition(condition, true);
                for place in owned_bindings {
                    self.flow_ctx.set_place_state(*place, VarState::Alive);
                }
                *body
            }
            TypedWhenArm::Is {
                pattern,
                body,
                owned_bindings,
                ..
            } => {
                if let Some(subject) = subject {
                    let condition = crate::typed::TypedCondition::Is {
                        subject,
                        pattern: pattern.clone(),
                    };
                    let values = self.walk_condition_values(&condition);
                    for (left, right, strict) in self.condition_facts(&condition, true, &values) {
                        if strict {
                            self.flow_ctx.constraint_env.known_lt.push((left, right));
                        } else {
                            self.flow_ctx.constraint_env.known_le.push((left, right));
                        }
                    }
                }
                for place in owned_bindings {
                    self.flow_ctx.set_place_state(*place, VarState::Alive);
                }
                if let Some(subject) = subject {
                    self.exclude_pattern_variants(subject, &pattern.value, true);
                    self.remember_payload_pattern(subject, &pattern.value, owned_bindings);
                }
                if let Some(subject) = subject
                    && !self.typed.borrows_value(subject)
                    && !self.pattern_discards_owned_fields(&pattern.value, subject)
                    && (self.typed.temporary_owner(subject).is_some()
                        || pattern_moves_subject(&pattern.value)
                        || pattern_moves_empty_variant(self.typed, &pattern.value, subject))
                {
                    self.validate_unpack_authority(subject, site);
                    mark_consumed_if_local_ref(self.typed, subject, self.flow_ctx);
                    if let Some(source) =
                        self.typed.exprs.target_group_of(subject).cloned().flatten()
                    {
                        self.ref_tracker
                            .permanently_invalidate(&source, self.flow_ctx);
                    }
                }
                *body
            }
            TypedWhenArm::Else(body, _) => *body,
        }
    }

    fn walk_projection_base(&mut self, expr: ExprId) {
        let kind = self.typed.exprs.kind[expr.as_usize()].clone();
        match kind {
            TypedExprKind::FnCall { args: None, .. } | TypedExprKind::SelfRef { .. } => {
                let projecting = std::mem::replace(&mut self.projecting, true);
                self.walk(expr);
                self.projecting = projecting;
            }
            TypedExprKind::AggregateGet { base, .. }
            | TypedExprKind::UnionPayload { expr: base, .. } => {
                self.walk_projection_base(base);
                self.check_place_use(expr, false);
            }
            _ => self.walk(expr),
        }
    }

    fn check_place_use(&mut self, expr: ExprId, whole: bool) {
        self.record_capture_read(expr, whole);
        self.check_place_use_at(expr, whole, expr);
    }

    fn check_place_use_at(&mut self, expr: ExprId, whole: bool, site: ExprId) {
        let Some(place) = self.typed.exprs.place_of(expr).flatten() else {
            return;
        };
        let state = ownership_state(self.typed, self.flow_ctx, place, whole);
        let (code, reason) = match state {
            Some(VarState::Consumed) => (
                // type-use-of-moved-value
                DiagnosticCode::Type(TypeCode::Ownership(OwnershipCode::UseOfMovedValue)),
                "has consumed contents",
            ),
            Some(VarState::MaybeConsumed) => (
                // type-use-of-maybe-moved-value
                DiagnosticCode::Type(TypeCode::Ownership(OwnershipCode::UseOfMaybeMovedValue)),
                "may have consumed contents",
            ),
            Some(VarState::Invalidated) => (
                // type-use-of-invalidated-reference
                DiagnosticCode::Type(TypeCode::Reference(ReferenceCode::UseAfterInvalidation)),
                "has been invalidated",
            ),
            Some(VarState::MaybeInvalidated) => (
                DiagnosticCode::Type(TypeCode::Reference(
                    // type-use-of-maybe-invalidated-reference
                    ReferenceCode::UseAfterPossibleInvalidation,
                )),
                "may have been invalidated",
            ),
            _ => return,
        };
        if self.typed.exprs.facts.semantic_facts.flaws[site.as_usize()]
            .iter()
            .any(|flaw| flaw.code == code)
        {
            return;
        }
        let name = self.typed.places[place.0 as usize].name;
        let diagnostic = Diagnostic::new(code, format!("value `{name}` {reason}"))
            .with_arg("name", name.as_str().to_string())
            .at_span_id(self.typed.exprs.span[site.index()], &self.typed.span_table);
        self.typed.exprs.facts.semantic_facts.flaws[site.as_usize()].push(diagnostic);
    }

    fn walk(&mut self, expr_id: ExprId) {
        if self.when_subjects.contains(&expr_id) {
            return;
        }
        let idx = expr_id.index();
        let Some(kind) = self.typed.exprs.kind_of(expr_id).cloned() else {
            return;
        };
        self.check_array_reference_contract(expr_id);

        if self.typed.local_callable_parameters.contains_key(&expr_id) {
            self.walk_local_callable(expr_id);
            return;
        }

        let is_call = matches!(&kind, TypedExprKind::FnCall { .. });
        match kind {
            TypedExprKind::Block { statements, value } => {
                for statement in statements {
                    self.walk(statement);
                    self.validate_discarded_result(statement);
                    if crate::analysis::expression_returns(self.typed.resolved_program(), statement)
                    {
                        return;
                    }
                }
                if let Some(value) = value {
                    self.walk(value);
                }
            }
            TypedExprKind::Return {
                value,
                postcondition,
            } => {
                if let Some(value) = value {
                    self.walk(value);
                    mark_consumed_if_local_ref(self.typed, value, self.flow_ctx);
                }
                if let Some(proof) = &postcondition {
                    self.verify_postcondition(proof, Some(expr_id));
                }
                self.validate_terminal_exit(Some(expr_id));
            }
            TypedExprKind::Invalid | TypedExprKind::Todo => {}
            TypedExprKind::Emit {
                state,
                specification,
                operands,
                record,
                ..
            } => {
                self.walk(specification);
                self.walk(record);
                if let crate::assembly::AssemblyState::Checked(assembly) = state {
                    if !assembly.memory.writes.is_empty() {
                        self.snapshot_fields(expr_id, None, &[]);
                    }
                    for (pointer, _) in assembly.memory.writes {
                        let Some((_, operand)) =
                            operands.iter().find(|(name, _)| name.as_str() == pointer)
                        else {
                            continue;
                        };
                        if !self.typed.exprs.ty_of(*operand).is_some_and(Ty::is_ref) {
                            continue;
                        }
                        if let Some(targets) = self
                            .typed
                            .exprs
                            .target_group_of(*operand)
                            .cloned()
                            .flatten()
                        {
                            self.ref_tracker
                                .permanently_invalidate(&targets, self.flow_ctx);
                        }
                    }
                }
            }
            TypedExprKind::Domain(_)
            | TypedExprKind::Lit(_)
            | TypedExprKind::Rematerialize { .. } => {}
            TypedExprKind::Binary { lhs, rhs, .. }
            | TypedExprKind::InvalidOperator { lhs, rhs, .. } => {
                self.walk(lhs);
                self.walk(rhs);
            }
            TypedExprKind::IntrinsicCall { op, args } => {
                for arg in &args {
                    self.walk(*arg);
                }
                if matches!(op, crate::intrinsic::IntrinsicOp::AddressStore) {
                    self.snapshot_fields(expr_id, None, &[]);
                }
                let all_operands_anonymous_int = args.iter().all(|arg| {
                    let arg_ty = self.typed.exprs.ty_of(*arg).unwrap_or(&Ty::Unit);
                    arg_ty.is_int()
                        && arg_ty
                            .named_instance_stripping_reference_wrappers()
                            .is_none()
                });
                let result_ty = self.typed.exprs.ty_of(expr_id).unwrap_or(&Ty::Unit);
                let is_anonymous_result_int = matches!(result_ty, Ty::AnonymousInteger { .. });
                if all_operands_anonymous_int
                    && is_anonymous_result_int
                    && !self.is_constant_definition
                {
                    self.typed.exprs.facts.const_facts.const_value[idx] = None;
                }
            }
            TypedExprKind::FnCall {
                target,
                args,
                substituted_ty,
                ..
            } => {
                if args.as_ref().is_none_or(|a| a.is_empty())
                    && self.typed.exprs.place_of(expr_id).flatten().is_some()
                    && let Some(cv) = self.local_const_values.get(&target.0).cloned()
                {
                    self.typed.exprs.facts.const_facts.const_value[idx] = Some(cv);
                }
                if args.as_ref().is_none_or(|a| a.is_empty())
                    && self.typed.temporary_owner(expr_id).is_none()
                {
                    self.record_capture_read(expr_id, !self.projecting);
                    match self
                        .typed
                        .exprs
                        .place_of(expr_id)
                        .flatten()
                        .and_then(|place| {
                            ownership_state(self.typed, self.flow_ctx, place, !self.projecting)
                        }) {
                        Some(VarState::Consumed) => {
                            self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                                // type-use-of-moved-value
                                Diagnostic::new(
                                    OwnershipCode::UseOfMovedValue,
                                    format!("use of moved value `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                            self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                                // type-use-after-move
                                Diagnostic::new(
                                    OwnershipCode::UseAfterMove,
                                    format!("use of moved value `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        Some(VarState::MaybeConsumed) => {
                            self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                                // type-use-of-maybe-moved-value
                                Diagnostic::new(
                                    OwnershipCode::UseOfMaybeMovedValue,
                                    format!(
                                        "value `{}` may have been moved on another control-flow path",
                                        target.0.as_str()
                                    ),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        Some(VarState::Invalidated) => {
                            self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                                // type-use-of-invalidated-reference
                                Diagnostic::new(
                                    ReferenceCode::UseAfterInvalidation,
                                    format!("use of invalidated reference `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        Some(VarState::MaybeInvalidated) => {
                            self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                                // type-use-of-maybe-invalidated-reference
                                Diagnostic::new(
                                    ReferenceCode::UseAfterPossibleInvalidation,
                                    format!(
                                        "reference `{}` may have been invalidated on another control-flow path",
                                        target.0.as_str()
                                    ),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        Some(VarState::Declared) => {
                            self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                                // type-use-before-assign
                                Diagnostic::new(
                                    BindingCode::UseBeforeAssignment,
                                    format!("use before assign `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                            // type-use-before-initialization
                            let mut diagnostic = Diagnostic::new(
                                BindingCode::UseBeforeInitialization,
                                format!("use of uninitialized local `{}`", target.0.as_str()),
                            )
                            .with_arg("name", target.0.as_str().to_string());
                            let declaration = self
                                .typed
                                .defs
                                .get(&target)
                                .or_else(|| {
                                    let place = self.typed.exprs.place_of(expr_id).flatten()?;
                                    let binder = self.typed.places.get(place.0 as usize)?.binder;
                                    self.typed
                                        .defs
                                        .values()
                                        .find(|bind| bind.dependent_binder == binder)
                                })
                                .or_else(|| {
                                    self.typed
                                        .defs
                                        .values()
                                        .find(|bind| bind.name == target.0 && bind.unassigned_decl)
                                });
                            if let Some(bind) = declaration {
                                diagnostic.related_mut().push(RelatedSpan {
                                    span: self.typed.span_table.get(bind.name_span),
                                    label: "declared without an initializer".to_string(),
                                });
                            } else if let Some(place) = self.typed.exprs.place_of(expr_id).flatten()
                                && let Some((expression, _)) = self
                                    .typed
                                    .exprs
                                    .place_entries()
                                    .find(|(_, candidate)| *candidate == Some(place))
                                && let Some(span) = self.typed.exprs.span_of(expression)
                            {
                                diagnostic.related_mut().push(RelatedSpan {
                                    span: self.typed.span_table.get(span),
                                    label: "declared without an initializer".to_string(),
                                });
                            }
                            if let Some(place) = self.typed.exprs.place_of(expr_id).flatten() {
                                for version in self
                                    .typed
                                    .place_versions
                                    .iter()
                                    .filter(|version| version.place == place)
                                {
                                    if let crate::typed::PlaceVersionOrigin::Rebind {
                                        value, ..
                                    } = version.origin
                                        && let Some(span) = self.typed.exprs.span_of(value)
                                    {
                                        diagnostic.related_mut().push(RelatedSpan {
                                            span: self.typed.span_table.get(span),
                                            label: "initialized on this predecessor".to_string(),
                                        });
                                    }
                                }
                            }
                            self.typed.exprs.facts.semantic_facts.flaws[idx].push(diagnostic);
                        }
                        _ => {}
                    }
                }
                if let Some(arg_ids) = args {
                    let local_callable = self.typed.local_callable_calls.get(&expr_id).copied();
                    let signature = local_callable
                        .is_none()
                        .then(|| self.callable_signatures.get(&target).cloned())
                        .flatten();
                    let local_parameters = local_callable.and_then(|definition| {
                        self.typed
                            .local_callable_parameters
                            .get(&definition)
                            .cloned()
                    });
                    let conventions = if let Some(parameters) = &local_parameters {
                        parameters
                            .iter()
                            .map(|parameter| parameter.convention)
                            .collect()
                    } else {
                        signature
                            .as_ref()
                            .map(|signature| signature.param_conventions.clone())
                            .unwrap_or_default()
                    };
                    let mut argument_values = Vec::with_capacity(arg_ids.len());
                    for (slot, arg_id) in arg_ids.iter().enumerate() {
                        self.walk(*arg_id);
                        argument_values.push(self.capture_scalar_argument(expr_id, slot, *arg_id));
                    }
                    if let Some(signature) = &signature {
                        self.check_proposition_arguments(
                            expr_id,
                            &signature.params,
                            &arg_ids,
                            &argument_values,
                        );
                    }
                    if let Some(parameters) = &local_parameters {
                        let parameters = parameters
                            .iter()
                            .map(|parameter| (parameter.name, parameter.ty.clone()))
                            .collect::<Vec<_>>();
                        self.check_proposition_arguments(
                            expr_id,
                            &parameters,
                            &arg_ids,
                            &argument_values,
                        );
                    }
                    self.check_reference_reads(expr_id);
                    self.check_consuming_argument_aliases(expr_id, &arg_ids, &conventions);
                    for (param_idx, arg_id) in arg_ids.iter().enumerate() {
                        let convention = conventions.get(param_idx);
                        if !matches!(
                            convention,
                            Some(&ParamConvention::Own | &ParamConvention::Terminal)
                        ) {
                            continue;
                        }
                        let parameter_type = local_parameters
                            .as_ref()
                            .and_then(|parameters| parameters.get(param_idx))
                            .map(|parameter| &parameter.ty)
                            .or_else(|| {
                                signature
                                    .as_ref()
                                    .and_then(|signature| signature.params.get(param_idx))
                                    .map(|(_, ty)| ty)
                            });
                        let expects_value =
                            parameter_type.is_some_and(|ty| !is_reference_type(self.typed, ty));
                        if expects_value
                            && self.typed.borrows_value(*arg_id)
                            && self
                                .typed
                                .exprs
                                .ty_of(*arg_id)
                                .is_some_and(|ty| is_reference_type(self.typed, ty))
                        {
                            // type-move-out-of-borrow
                            let diagnostic = Diagnostic::new(
                                OwnershipCode::MoveOutOfBorrow,
                                "an owned parameter requires ownership, not a borrow",
                            )
                            .at_span_id(
                                self.typed.exprs.span[arg_id.index()],
                                &self.typed.span_table,
                            );
                            self.typed.exprs.facts.semantic_facts.flaws[arg_id.index()]
                                .push(diagnostic);
                            continue;
                        }
                        if matches!(
                            self.typed.exprs.kind_of(*arg_id),
                            Some(TypedExprKind::Ref(_))
                        ) {
                            continue;
                        }
                        if !expr_is_rematerializable(self.typed, *arg_id) {
                            self.check_place_use(*arg_id, true);
                            let place = self.typed.exprs.place_of(*arg_id).flatten();
                            if let Some(place) = place
                                && Some(place) == self.replacing
                                && !self.typed.call_argument_is_shared(*arg_id)
                            {
                                self.flow_ctx.set_place_state(place, VarState::Consumed);
                            } else {
                                mark_consumed_if_local_ref(self.typed, *arg_id, self.flow_ctx);
                            }
                            if let Some(source) =
                                self.typed.exprs.target_group_of(*arg_id).cloned().flatten()
                            {
                                if place.is_some() && place == self.replacing {
                                    self.ref_tracker.invalidate_replaced(&source, self.flow_ctx);
                                } else {
                                    self.ref_tracker
                                        .permanently_invalidate(&source, self.flow_ctx);
                                }
                            }
                        }
                    }
                    if let Some(callable) = local_callable {
                        if let Some(parameters) = &local_parameters {
                            let parameters = parameters
                                .iter()
                                .map(|parameter| (parameter.name, parameter.ty.clone()))
                                .collect::<Vec<_>>();
                            self.check_dependent_array_arguments(
                                &parameters,
                                &arg_ids,
                                &argument_values,
                            );
                        }
                        self.apply_local_call_ownership(expr_id, callable);
                        self.apply_local_call_writes(expr_id, callable);
                    }
                    if let Some(signature) = signature {
                        self.check_dependent_array_arguments(
                            &signature.params,
                            &arg_ids,
                            &argument_values,
                        );
                        let refinement_flaws = validate_call_refinements(
                            self.typed,
                            CallRefinementInputs {
                                args: &arg_ids,
                                argument_values: &argument_values,
                                signature: &signature,
                                substituted_return: substituted_ty.as_ref(),
                            },
                            &self.flow_ctx.constraint_env,
                            self.callable_signatures,
                            self.local_const_values,
                        );
                        for flaw in refinement_flaws {
                            let flaws = &mut self.typed.exprs.facts.semantic_facts.flaws[idx];
                            if !flaws.contains(&flaw) {
                                flaws.push(flaw);
                            }
                        }
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
                            self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                                // type-invalid-derived-target-group-argument
                                Diagnostic::new(
                                    TargetGroupCode::InvalidDerivedArgument,
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
                            self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                                // type-overlapping-target-group-arguments
                                Diagnostic::new(
                                    TargetGroupCode::OverlappingArguments,
                                    format!(
                                        "arguments for target groups `{left_name}` and `{right_name}` may overlap, but one of the groups is mutated"
                                    ),
                                )
                                .with_arg("left_group", left_name)
                                .with_arg("right_group", right_name),
                            );
                        }
                        for applied in self.applied_effect_targets(
                            &arg_ids,
                            &signature,
                            &signature.effects.consumes,
                        ) {
                            self.ref_tracker.permanently_invalidate(
                                &ReferenceTargetSet::singleton(applied.target),
                                self.flow_ctx,
                            );
                        }
                        for applied in self.applied_effect_targets(
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
                                    .invalidate_replaced(&targets, self.flow_ctx);
                            }
                        }
                        self.apply_scalar_call_effects(
                            expr_id,
                            &arg_ids,
                            &argument_values,
                            &signature,
                        );
                        self.remember_returned_record(expr_id, &argument_values, &signature);
                        self.apply_postconditions(expr_id, &arg_ids, &argument_values, &signature);
                    }
                } else if self.typed.exprs.place_of(expr_id).flatten().is_none()
                    && let Some(signature) = self.callable_signatures.get(&target).cloned()
                {
                    self.apply_scalar_call_effects(expr_id, &[], &[], &signature);
                    self.apply_postconditions(expr_id, &[], &[], &signature);
                }
            }
            TypedExprKind::TagCall {
                variant_id,
                args,
                field_names,
                ..
            } => {
                let mut argument_values = Vec::new();
                if let Some(arg_ids) = &args {
                    for (slot, arg_id) in arg_ids.iter().copied().enumerate() {
                        self.walk(arg_id);
                        argument_values.push(self.capture_scalar_argument(expr_id, slot, arg_id));
                        if !expr_is_rematerializable(self.typed, arg_id) {
                            mark_consumed_if_local_ref(self.typed, arg_id, self.flow_ctx);
                        }
                    }
                }
                // Check refinements for this TagCall
                if let Some(arguments) = &args {
                    self.check_dependent_record_arguments(
                        expr_id,
                        arguments,
                        &field_names,
                        &argument_values,
                    );
                }
                self.check_tag_refinements(expr_id, &variant_id, args.as_deref(), &field_names);
            }
            TypedExprKind::Bind {
                name,
                stmts,
                body,
                unassigned,
            } => {
                self.check_array_length_mutation(expr_id, name);
                for stmt in stmts {
                    if stmt.index() == idx {
                        continue;
                    }
                    self.walk(stmt);
                }
                if !unassigned && body.index() != idx {
                    self.walk(body);
                    self.validate_proven_integer_cast(expr_id, body);
                    let expected = self.typed.exprs.ty[idx].clone();
                    self.check_dependent_array_argument(
                        &expected,
                        body,
                        &crate::subst::DepSubst::new(),
                    );
                }
                self.flow_ctx.constraint_env.forget(name);
                let bind_const = if unassigned {
                    None
                } else {
                    self.typed
                        .exprs
                        .const_value_of(body)
                        .cloned()
                        .flatten()
                        .or_else(|| self.typed.exprs.facts.const_facts.const_value[idx].clone())
                };
                self.typed.exprs.facts.const_facts.const_value[idx] = bind_const.clone();
                if let Some(cv) = bind_const {
                    self.const_bindings.insert(name, true);
                    self.local_const_values.insert(name, cv);
                } else {
                    self.const_bindings.remove(&name);
                    self.local_const_values.remove(&name);
                }
                let place = self.typed.exprs.place_of(expr_id).flatten();
                self.remember_record_binding(name, body, place);
                if !unassigned {
                    self.remember_record_fields(name, body);
                    self.remember_record_alias(name, body);
                    if let Some(ty) = self.flow_ctx.binding_types.get(&name) {
                        record_constraints::assume_integer_bounds(
                            self.typed,
                            ty,
                            NormalExpr::Var(name),
                            &mut self.flow_ctx.constraint_env,
                        );
                        record_constraints::assume_record_constraints(
                            self.typed,
                            self.callable_signatures,
                            name,
                            ty,
                            &mut self.flow_ctx.constraint_env,
                        );
                    }
                }
                if !unassigned
                    && super::symbolic_values::scalar_type(
                        self.typed,
                        &self.typed.exprs.ty[expr_id.index()],
                    )
                    && let Some(value) = super::symbolic_values::normal_expr(
                        self.typed,
                        Some(self.callable_signatures),
                        self.local_const_values,
                        body,
                    )
                {
                    self.flow_ctx
                        .constraint_env
                        .assume(&Predicate::Eq(NormalExpr::Var(name), value));
                }
                if unassigned {
                    if let Some(place) = place {
                        self.flow_ctx.set_place_state(place, VarState::Declared);
                    }
                } else {
                    if let Some(place) = place {
                        self.flow_ctx.set_place_state(place, VarState::Alive);
                    }
                    let is_reference =
                        matches!(self.typed.exprs.kind_of(body), Some(TypedExprKind::Ref(_)))
                            || self
                                .typed
                                .exprs
                                .ty_of(body)
                                .is_some_and(|ty| is_reference_type(self.typed, ty));
                    if is_reference && let Some(place) = place {
                        let targets = self.typed.exprs.target_group_of(body).cloned().flatten();
                        self.ref_tracker.register(place, targets);
                    } else if !is_reference
                        && !expr_is_rematerializable(self.typed, body)
                        && (self.typed.exprs.ty_of(body).is_some_and(|ty| {
                            self.typed.borrowed_value_needs_ownership(ty)
                                || self.pending_terminal_obligation(ty, None)
                        }) || matches!(
                            self.typed.exprs.kind_of(body),
                            Some(TypedExprKind::Deref(_))
                        ))
                    {
                        mark_consumed_if_local_ref(self.typed, body, self.flow_ctx);
                    }
                }
            }
            TypedExprKind::Reassign { name, value, .. } => {
                self.check_array_length_mutation(expr_id, name);
                self.walk_replacement_value(expr_id, value);
                let (scalar_value, assignment_snapshot) =
                    if self.flow_ctx.scalar_bindings.contains(&name) {
                        let value = self.flow_ctx.scalar_values.get(&value).cloned();
                        let targets = self.flow_ctx.record_targets.get(&name).cloned();
                        let snapshots = self.snapshot_fields(
                            expr_id,
                            targets.as_ref(),
                            &value.iter().cloned().collect::<Vec<_>>(),
                        );
                        (
                            value.map(|value| snapshots.apply_to_normal(&value)),
                            snapshots,
                        )
                    } else {
                        (None, crate::subst::DepSubst::new())
                    };
                if self.is_local_reference_binding(expr_id) {
                    self.snapshot_reference_binding(expr_id, name);
                } else if !self.flow_ctx.scalar_bindings.contains(&name) {
                    self.forget_record_facts();
                }
                self.flow_ctx.constraint_env.forget(name);
                self.local_const_values.remove(&name);
                self.const_bindings.remove(&name);
                self.remember_record_binding(
                    name,
                    value,
                    self.typed.exprs.place_of(expr_id).flatten(),
                );
                self.remember_record_alias(name, value);
                let Some(target_group) = self
                    .typed
                    .exprs
                    .target_group_of(ExprId(idx as u32))
                    .and_then(|targets| targets.as_ref())
                    .cloned()
                else {
                    if self
                        .typed
                        .defs
                        .contains_key(&crate::typed::DefId::named(name))
                    {
                        self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                            // type-rebind-crosses-callable-boundary
                            Diagnostic::new(
                                BindingCode::RebindCrossesCallableBoundary,
                                format!(
                                    "cannot rebind callable `{}` across scope boundary",
                                    name.as_str()
                                ),
                            )
                            .with_arg("name", name.as_str().to_string()),
                        );
                    } else {
                        self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                            // type-rebind-target-not-found
                            Diagnostic::new(
                                BindingCode::RebindTargetNotFound,
                                format!("unknown rebind target `{}`", name.as_str()),
                            )
                            .with_arg("name", name.as_str().to_string()),
                        );
                    }
                    return;
                };

                let target_place = self.typed.exprs.place_of(expr_id).flatten();
                let target_empty = target_place
                    .and_then(|place| {
                        ownership_state(self.typed, self.flow_ctx, place, !self.projecting)
                    })
                    .is_some_and(|state| matches!(state, VarState::Declared | VarState::Consumed));
                let target_ty = self
                    .typed
                    .exprs
                    .place_of(expr_id)
                    .flatten()
                    .and_then(|place| self.typed.places.get(place.0 as usize))
                    .map(|place| &place.ty)
                    .or_else(|| self.local_var_types.get(&name))
                    .cloned();
                if !target_empty
                    && let Some(ty) = target_ty.as_ref()
                    && !ty.is_structurally_discardable()
                    && !target_place.is_some_and(|place| {
                        self.typed.places[place.0 as usize]
                            .parent
                            .is_some_and(|parent| {
                                self.typed.borrowed_parameter_referent(parent).is_some()
                                    && self
                                        .typed
                                        .type_registry
                                        .resolved_definition_for_type(ty)
                                        .is_structurally_discardable()
                            })
                    })
                    && !is_reference_type(self.typed, ty)
                    && !is_explicit_rebind_consumption(self.typed, value, name)
                {
                    self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                        // type-rebind-would-discard-value
                        Diagnostic::new(
                            BindingCode::RebindWouldDiscardValue,
                            format!(
                                "replacing `{}` requires consuming or transferring its previous value",
                                name.as_str()
                            ),
                        )
                        .with_arg("name", name.as_str().to_string()),
                    );
                }
                for target in target_group.iter() {
                    self.ref_tracker.replace(target, self.flow_ctx);
                }
                if !expr_is_rematerializable(self.typed, value)
                    && self
                        .typed
                        .exprs
                        .ty_of(value)
                        .is_some_and(|ty| !is_reference_type(self.typed, ty))
                {
                    mark_consumed_if_local_ref(self.typed, value, self.flow_ctx);
                }
                if let Some(place) = target_place {
                    self.flow_ctx.set_place_state(place, VarState::Alive);
                }

                let validity = target_ty.as_ref().and_then(|ty| {
                    self.typed
                        .type_registry
                        .integer_validity_for_type(ty)
                        .and_then(|validity| validity.domain().storage_hull())
                        .and_then(|hull| {
                            Some((
                                ast::integer::to_i128(hull.min())?,
                                ast::integer::to_i128(hull.max())?,
                            ))
                        })
                        .or_else(|| ty.int_bounds().map(|bounds| (bounds.min, bounds.max)))
                });
                let divides_upper_bound = target_ty.as_ref().is_some_and(|ty| {
                    self.typed
                        .type_registry
                        .integer_validity_for_type(ty)
                        .or_else(|| ty.anonymous_integer_validity())
                        .is_some_and(|validity| refinement_divides_storage_max(validity.domain()))
                });
                let previous_value = assignment_snapshot.apply_to_normal(&NormalExpr::Var(name));
                let replacement_value = scalar_value.clone().or_else(|| {
                    expr_to_normal_expr(
                        &self.typed.exprs.kind,
                        &self.typed.exprs.facts.const_facts.const_value,
                        self.local_const_values,
                        value,
                    )
                    .map(|rhs| assignment_snapshot.apply_to_normal(&rhs))
                });
                let bounds_proven = validity.is_none_or(|(min, max)| {
                    scalar_value.as_ref().is_some_and(|value| {
                        self.flow_ctx
                            .constraint_env
                            .prove(&Predicate::Ge(value.clone(), min.into()), &HashMap::new())
                            == ProveResult::Proven
                            && self
                                .flow_ctx
                                .constraint_env
                                .prove(&Predicate::Le(value.clone(), max.into()), &HashMap::new())
                                == ProveResult::Proven
                    }) || rebind_result_within_integer_bounds(self.typed, value, min, max)
                        || replacement_value.as_ref().is_some_and(|rhs| {
                            rebind_rhs_within_integer_bounds(
                                rhs.clone(),
                                previous_value.clone(),
                                self.local_const_values,
                                min,
                                max,
                                divides_upper_bound,
                                &self.flow_ctx.constraint_env,
                            )
                        })
                });
                if !bounds_proven
                    || !self.rebind_preserves_integer_validity(
                        target_ty.as_ref(),
                        value,
                        replacement_value.as_ref(),
                        &previous_value,
                        validity.is_some() && bounds_proven,
                    )
                {
                    self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                        // type-rebind-violates-validity
                        Diagnostic::new(
                            BindingCode::RebindViolatesValidity,
                            format!(
                                "rebind `{}` does not preserve its integer validity",
                                name.as_str()
                            ),
                        )
                        .with_arg("name", name.as_str().to_string()),
                    );
                } else if let Some((min, max)) = validity {
                    self.flow_ctx
                        .constraint_env
                        .assume(&Predicate::Ge(NormalExpr::Var(name), min.into()));
                    self.flow_ctx
                        .constraint_env
                        .assume(&Predicate::Le(NormalExpr::Var(name), max.into()));
                }
                if let Some(value) = scalar_value {
                    self.flow_ctx
                        .constraint_env
                        .assume(&Predicate::Eq(NormalExpr::Var(name), value));
                }
            }
            TypedExprKind::TakePtr(inner) | TypedExprKind::Ref(inner) => {
                if self
                    .typed
                    .exprs
                    .ty_of(expr_id)
                    .is_some_and(Ty::is_mutable_ref)
                {
                    self.validate_constrained_field_borrow(expr_id, inner);
                    if let Some(TypedExprKind::FnCall {
                        target, args: None, ..
                    }) = self.typed.exprs.kind_of(inner)
                    {
                        self.check_array_length_mutation(expr_id, target.0);
                    }
                }
                if self
                    .typed
                    .exprs
                    .ty_of(expr_id)
                    .is_some_and(Ty::is_mutable_ref)
                    && self.typed.shared_owned_storage_access(inner)
                {
                    // type-mutable-borrow-from-shared
                    let diagnostic = Diagnostic::new(
                        OwnershipCode::MutableBorrowFromShared,
                        "shared owned storage cannot provide a mutable element borrow",
                    )
                    .at_span_id(self.typed.exprs.span[idx], &self.typed.span_table);
                    self.typed.exprs.facts.semantic_facts.flaws[idx].push(diagnostic);
                }
                self.walk(inner);
                if matches!(kind, TypedExprKind::TakePtr(_)) {
                    self.expose_raw_address(inner);
                }
            }
            TypedExprKind::When(when_expr) => {
                if let Some(subject_id) = when_expr.subject {
                    self.walk(subject_id);
                    self.when_subjects.push(subject_id);
                }
                let subject_value = when_expr.subject.and_then(|subject_id| {
                    self.typed
                        .exprs
                        .const_value_of(subject_id)
                        .and_then(Option::as_ref)
                        .cloned()
                });
                let mut selected_body = None;
                let target_selection = when_expr.subject.is_some_and(|subject| {
                    matches!(
                        self.typed.exprs.kind_of(subject),
                        Some(TypedExprKind::TargetField(_))
                    )
                });
                if let Some(subject_value) = subject_value.as_ref().filter(|_| {
                    !when_expr
                        .arms
                        .iter()
                        .any(|arm| matches!(arm, TypedWhenArm::Cond { .. }))
                }) {
                    for arm in &when_expr.arms {
                        match arm {
                            TypedWhenArm::Is { pattern, body, .. } => {
                                let matches =
                                    crate::analysis::pattern::pattern_matches_with_tag_types(
                                        &pattern.value,
                                        subject_value,
                                        &self.typed.tag_types,
                                        &self.typed.type_registry,
                                    ) || super::lower_exprs::pattern_value_const(
                                        &pattern.value,
                                        self.typed,
                                    )
                                    .as_ref()
                                    .is_some_and(|cv| *cv == *subject_value);
                                if matches {
                                    selected_body = Some(*body);
                                    break;
                                }
                                if !target_selection {
                                    self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                                        // type-unreachable-when-arm
                                        Diagnostic::new(
                                            PatternCode::ArmUnreachable,
                                            "when arm is unreachable",
                                        ),
                                    );
                                }
                            }
                            TypedWhenArm::Else(body, _) => {
                                selected_body = Some(*body);
                                break;
                            }
                            TypedWhenArm::Cond { .. } => {}
                        }
                    }
                } else if when_expr.subject.is_none()
                    && let Some(arm) = when_expr.arms.first()
                {
                    match arm {
                        TypedWhenArm::Cond { .. } => {}
                        TypedWhenArm::Else(body, _) => {
                            selected_body = Some(*body);
                        }
                        TypedWhenArm::Is { .. } => {}
                    }
                }
                if let Some(body) = selected_body {
                    if let Some((index, arm)) =
                        when_expr
                            .arms
                            .iter()
                            .enumerate()
                            .find(|(_, arm)| match arm {
                                TypedWhenArm::Is {
                                    body: candidate, ..
                                }
                                | TypedWhenArm::Cond {
                                    body: candidate, ..
                                }
                                | TypedWhenArm::Else(candidate, _) => *candidate == body,
                            })
                    {
                        self.enter_when_arm(
                            arm,
                            when_expr.subject,
                            &when_expr.arms[..index],
                            expr_id,
                        );
                    }
                    self.walk(body);
                    self.remember_conditional_record(expr_id, body);
                    let expected = self.typed.exprs.ty[idx].clone();
                    self.check_dependent_array_argument(
                        &expected,
                        body,
                        &crate::subst::DepSubst::new(),
                    );
                    if self.typed.exprs.ty[idx] == Ty::Unit
                        && !crate::analysis::expression_returns(self.typed.resolved_program(), body)
                    {
                        self.validate_discarded_result(body);
                    } else if !self.typed.borrows_value(body) {
                        mark_consumed_if_local_ref(self.typed, body, self.flow_ctx);
                    }
                    self.typed.exprs.facts.const_facts.const_value[idx] = self
                        .typed
                        .exprs
                        .const_value_of(body)
                        .cloned()
                        .unwrap_or(None);
                } else {
                    let snapshot = self.snapshot();
                    let mut branch_flows = Vec::with_capacity(when_expr.arms.len());
                    let mut branch_refs = Vec::with_capacity(when_expr.arms.len());
                    for (arm_index, arm) in when_expr.arms.iter().enumerate() {
                        self.restore(snapshot.clone());
                        let body = self.enter_when_arm(
                            arm,
                            when_expr.subject,
                            &when_expr.arms[..arm_index],
                            expr_id,
                        );
                        self.walk(body);
                        self.remember_conditional_record(expr_id, body);
                        let expected = self.typed.exprs.ty[idx].clone();
                        self.check_dependent_array_argument(
                            &expected,
                            body,
                            &crate::subst::DepSubst::new(),
                        );
                        if self.typed.exprs.ty[idx] == Ty::Unit
                            && !crate::analysis::expression_returns(
                                self.typed.resolved_program(),
                                body,
                            )
                        {
                            self.validate_discarded_result(body);
                        } else if !self.typed.borrows_value(body) {
                            mark_consumed_if_local_ref(self.typed, body, self.flow_ctx);
                        }
                        if !crate::analysis::expression_returns(self.typed.resolved_program(), body)
                        {
                            branch_flows.push(self.flow_ctx.clone());
                            branch_refs.push(self.ref_tracker.clone());
                        }
                    }
                    if !when_expr
                        .arms
                        .iter()
                        .any(|arm| matches!(arm, TypedWhenArm::Else(..)))
                        && !crate::analysis::when_is_exhaustive(
                            when_expr
                                .subject
                                .and_then(|id| self.typed.exprs.ty_of(id))
                                .map(|ty| self.typed.type_registry.resolved_definition_for_type(ty))
                                .as_ref(),
                            &when_expr.arms,
                            &self.typed.type_registry,
                        )
                    {
                        self.restore(snapshot.clone());
                        self.reject_when_arms(&when_expr.arms, when_expr.subject);
                        branch_flows.push(self.flow_ctx.clone());
                        branch_refs.push(self.ref_tracker.clone());
                    }
                    let incoming_flow = snapshot.flow_ctx.clone();
                    let incoming_refs = snapshot.ref_tracker.clone();
                    *self.flow_ctx = incoming_flow.clone();
                    self.flow_ctx.merge_branch_states(&branch_flows);
                    if let Some(first) = branch_flows.first() {
                        let mut constraints = first.constraint_env.clone();
                        constraints.known_eq.retain(|fact| {
                            branch_flows
                                .iter()
                                .all(|branch| branch.constraint_env.known_eq.contains(fact))
                        });
                        constraints.known_lt.retain(|fact| {
                            branch_flows
                                .iter()
                                .all(|branch| branch.constraint_env.known_lt.contains(fact))
                        });
                        constraints.known_le.retain(|fact| {
                            branch_flows
                                .iter()
                                .all(|branch| branch.constraint_env.known_le.contains(fact))
                        });
                        constraints.known_divisible.retain(|fact| {
                            branch_flows
                                .iter()
                                .all(|branch| branch.constraint_env.known_divisible.contains(fact))
                        });
                        self.flow_ctx.constraint_env = constraints;
                    }
                    self.close_pattern_scopes(&incoming_flow, &branch_flows);
                    *self.ref_tracker = incoming_refs;
                    self.ref_tracker.merge_branch_validity(&branch_refs);
                    self.const_bindings.clone_from(&snapshot.const_bindings);
                    self.local_const_values
                        .clone_from(&snapshot.local_const_values);
                }
                self.remember_conditional_bounds(expr_id, when_expr.subject, &when_expr.arms);
                if when_expr.subject.is_some() {
                    self.when_subjects.pop();
                }
            }
            TypedExprKind::If(if_expr) => {
                let values = self.walk_condition_values(&if_expr.condition);
                let comparison_facts = self.condition_facts(&if_expr.condition, true, &values);
                let rejected_facts = self.condition_facts(&if_expr.condition, false, &values);
                let saved_lt_len = self.flow_ctx.constraint_env.known_lt.len();
                let saved_le_len = self.flow_ctx.constraint_env.known_le.len();
                let saved_divisible_len = self.flow_ctx.constraint_env.known_divisible.len();
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
                let taken_ownership = self.condition_pattern_paths(&if_expr.condition, true);
                let rejected_ownership = self.condition_pattern_paths(&if_expr.condition, false);
                let mut taken = if_expr.stmts;
                taken.extend(if_expr.ret);
                self.walk_terminating_condition_branch(
                    &taken,
                    &taken_ownership,
                    if_expr.ret,
                    expr_id,
                    &if_expr.owned_bindings,
                    if_expr.postcondition.as_ref(),
                );
                // Restore constraint_env so facts don't leak past the if-block
                self.flow_ctx.constraint_env.known_lt.truncate(saved_lt_len);
                self.flow_ctx.constraint_env.known_le.truncate(saved_le_len);
                self.flow_ctx
                    .constraint_env
                    .known_divisible
                    .truncate(saved_divisible_len);
                for (lhs, rhs, is_lt) in rejected_facts {
                    if is_lt {
                        self.flow_ctx.constraint_env.known_lt.push((lhs, rhs));
                    } else {
                        self.flow_ctx.constraint_env.known_le.push((lhs, rhs));
                    }
                }
                self.apply_condition_ownership_join(&rejected_ownership, false);
            }
            TypedExprKind::Loop(typed_loop) => {
                let mut loop_expressions = Vec::new();
                let repeating = match &typed_loop.kind {
                    TypedLoopKind::While {
                        condition, stmts, ..
                    } => {
                        loop_expressions.extend(stmts);
                        condition.visit_subjects(&mut |subject| loop_expressions.push(subject));
                        true
                    }
                    TypedLoopKind::ForIn {
                        traversal:
                            crate::typed::SequenceTraversal::IntegerRange { stmts, .. }
                            | crate::typed::SequenceTraversal::RuntimeView { stmts, .. },
                        ..
                    } => {
                        loop_expressions.extend(stmts);
                        true
                    }
                    _ => false,
                };
                if repeating {
                    self.abstract_loop_state(expr_id, &loop_expressions);
                }
                let saved_lt_len = self.flow_ctx.constraint_env.known_lt.len();
                let saved_le_len = self.flow_ctx.constraint_env.known_le.len();
                let saved_divisible_len = self.flow_ctx.constraint_env.known_divisible.len();
                match typed_loop.kind {
                    TypedLoopKind::While {
                        ref condition,
                        ref stmts,
                        ref owned_bindings,
                    } => {
                        let values = self.walk_condition_values(condition);
                        for (lhs, rhs, is_lt) in self.condition_facts(condition, true, &values) {
                            if is_lt {
                                self.flow_ctx.constraint_env.known_lt.push((lhs, rhs));
                            } else {
                                self.flow_ctx.constraint_env.known_le.push((lhs, rhs));
                            }
                        }
                        self.walk_repeating_body(stmts, Some((condition, owned_bindings)));
                    }
                    TypedLoopKind::ForIn {
                        iterable,
                        ref traversal,
                    } => match traversal {
                        crate::typed::SequenceTraversal::IntegerRange {
                            variable,
                            start,
                            end,
                            stmts,
                        } => {
                            self.walk(iterable);
                            let lower = self.capture_scalar_argument(expr_id, 0, *start);
                            let upper = self.capture_scalar_argument(expr_id, 1, *end);
                            let constant = self.const_bindings.remove(variable);
                            let value = self.local_const_values.remove(variable);
                            self.flow_ctx.constraint_env.forget(*variable);
                            if let Some(lower) = lower {
                                self.flow_ctx
                                    .constraint_env
                                    .assume(&Predicate::Ge(NormalExpr::Var(*variable), lower));
                            }
                            if let Some(upper) = upper {
                                self.flow_ctx
                                    .constraint_env
                                    .assume(&Predicate::Le(NormalExpr::Var(*variable), upper));
                            }
                            self.walk_repeating_body(stmts, None);
                            if let Some(constant) = constant {
                                self.const_bindings.insert(*variable, constant);
                            }
                            if let Some(value) = value {
                                self.local_const_values.insert(*variable, value);
                            }
                        }
                        crate::typed::SequenceTraversal::RuntimeView { length, stmts, .. } => {
                            self.walk(iterable);
                            self.walk(*length);
                            self.validate_sequence_borrow(iterable, stmts);
                            self.walk_repeating_body(stmts, None);
                        }
                        crate::typed::SequenceTraversal::Fixed { iterations } => {
                            self.walk(iterable);
                            for statements in iterations {
                                self.walk_sequence_iteration(statements);
                            }
                        }
                        crate::typed::SequenceTraversal::Unresolved => {}
                    },
                };
                self.flow_ctx.constraint_env.known_lt.truncate(saved_lt_len);
                self.flow_ctx.constraint_env.known_le.truncate(saved_le_len);
                self.flow_ctx
                    .constraint_env
                    .known_divisible
                    .truncate(saved_divisible_len);
                if let TypedLoopKind::While { condition, .. } = &typed_loop.kind {
                    self.abstract_loop_state(expr_id, &loop_expressions);
                    let values = self.walk_condition_values(condition);
                    for (left, right, strict) in self.condition_facts(condition, false, &values) {
                        if strict {
                            self.flow_ctx.constraint_env.known_lt.push((left, right));
                        } else {
                            self.flow_ctx.constraint_env.known_le.push((left, right));
                        }
                    }
                } else if repeating {
                    self.abstract_loop_state(expr_id, &loop_expressions);
                }
            }
            TypedExprKind::AggregateGet { base, .. }
            | TypedExprKind::UnionPayload { expr: base, .. } => {
                self.walk_projection_base(base);
                self.record_capture_read(expr_id, true);
                if let Some(place) = self.typed.exprs.place_of(expr_id).flatten()
                    && let Some(state) = ownership_state(self.typed, self.flow_ctx, place, true)
                    && matches!(state, VarState::Consumed | VarState::MaybeConsumed)
                    && let Some(place_info) = self.typed.places.get(place.0 as usize)
                {
                    let (code, message) = match state {
                        VarState::Consumed => (
                            DiagnosticCode::Type(TypeCode::Ownership(
                                // type-use-of-moved-value
                                OwnershipCode::UseOfMovedValue,
                            )),
                            format!("use of moved value `{}`", place_info.name.as_str()),
                        ),
                        VarState::MaybeConsumed => (
                            DiagnosticCode::Type(TypeCode::Ownership(
                                // type-use-of-maybe-moved-value
                                OwnershipCode::UseOfMaybeMovedValue,
                            )),
                            format!(
                                "value `{}` may have been moved on another control-flow path",
                                place_info.name.as_str()
                            ),
                        ),
                        _ => unreachable!(),
                    };
                    self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                        Diagnostic::new(code, message)
                            .with_arg("name", place_info.name.as_str().to_string()),
                    );
                }
            }
            TypedExprKind::Destructure {
                tag_name,
                field_bindings,
                value,
                places,
            } => {
                self.walk(value);
                self.validate_unpack_authority(value, expr_id);
                for place in &places {
                    self.flow_ctx.set_place_state(*place, VarState::Alive);
                }
                self.remember_destructured_fields(value, &field_bindings, &places);
                let borrowed = self.typed.borrows_value(value);
                let value_ty = self
                    .typed
                    .exprs
                    .ty_of(value)
                    .and_then(|ty| reference_pointee_for_type(ty, Some(&self.typed.type_registry)))
                    .or_else(|| self.typed.exprs.ty_of(value).cloned())
                    .unwrap_or(Ty::Unit);
                if !borrowed {
                    mark_consumed_if_local_ref(self.typed, value, self.flow_ctx);
                    if let Some(source) = self.typed.exprs.target_group_of(value).cloned().flatten()
                    {
                        self.ref_tracker
                            .permanently_invalidate(&source, self.flow_ctx);
                    }
                }

                let mut bound_fields = HashSet::new();
                for (field_name, _) in field_bindings.iter() {
                    bound_fields.insert(*field_name);
                }

                let value_definition = self
                    .typed
                    .type_registry
                    .resolved_definition_for_type(&value_ty);
                let remainder_discardable = destructure_remainder_is_discardable(
                    &value_definition,
                    tag_name,
                    &bound_fields,
                );

                if !remainder_discardable {
                    self.typed.exprs.facts.semantic_facts.flaws[idx].push(
                        // type-pattern-would-discard-value
                        Diagnostic::new(
                            PatternCode::WouldDiscardValue,
                            format!(
                                "destructure leaves non-discardable unbound fields from `{}`",
                                tag_name.as_str()
                            ),
                        )
                        .with_arg("name", tag_name.as_str().to_string()),
                    );
                }
            }
            TypedExprKind::RecordSet {
                base, field, value, ..
            } => {
                self.walk_projection_base(base);
                self.walk_replacement_value(expr_id, value);
                self.validate_projected_write(expr_id, value);
                if let Some(place) = self.typed.exprs.place_of(expr_id).flatten() {
                    self.flow_ctx.set_place_state(place, VarState::Alive);
                }
                if let Some(base_targets) =
                    self.typed.exprs.target_group_of(base).cloned().flatten()
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
            TypedExprKind::AggregateSet {
                base, index, value, ..
            } => {
                self.walk_projection_base(base);
                self.walk_replacement_value(expr_id, value);
                self.validate_projected_write(expr_id, value);
                if let Some(place) = self.typed.exprs.place_of(expr_id).flatten() {
                    self.flow_ctx.set_place_state(place, VarState::Alive);
                }
                if let Some(base_targets) =
                    self.typed.exprs.target_group_of(base).cloned().flatten()
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
            TypedExprKind::Range { start, end } => {
                self.walk(start);
                self.walk(end);
            }
            TypedExprKind::FixedArrayAlloc { init, size } => {
                self.walk_repeated_array(expr_id, init, &size);
            }
            TypedExprKind::RecordValue(items)
            | TypedExprKind::ResultRow(items)
            | TypedExprKind::ArrayLit(items)
            | TypedExprKind::List(items) => {
                let record = matches!(self.typed.exprs.kind[idx], TypedExprKind::RecordValue(_));
                let mut values = Vec::new();
                for (slot, item_id) in items.iter().copied().enumerate() {
                    self.walk(item_id);
                    if record {
                        values.push(self.capture_scalar_argument(expr_id, slot, item_id));
                    }
                    if !expr_is_rematerializable(self.typed, item_id) {
                        mark_consumed_if_local_ref(self.typed, item_id, self.flow_ctx);
                    }
                }
                if record {
                    self.check_dependent_record_arguments(expr_id, &items, &[], &values);
                }
            }
            TypedExprKind::SliceGet { buf, start, length } => {
                self.check_slice_borrow(expr_id, buf, start, length);
                self.walk_projection_base(buf);
                self.walk(start);
                self.walk(length);
            }
            TypedExprKind::BufGet { buf, index } => {
                self.walk_projection_base(buf);
                self.walk(index);
                if self
                    .typed
                    .exprs
                    .place_of(expr_id)
                    .flatten()
                    .is_some_and(|place| self.typed.owns_projection(place))
                {
                    self.check_place_use(expr_id, true);
                }
            }
            TypedExprKind::BufSet {
                buf, index, value, ..
            } => {
                self.walk(buf);
                self.walk(index);
                self.walk_replacement_value(expr_id, value);
                self.validate_projected_write(expr_id, value);
                if !expr_is_rematerializable(self.typed, value) {
                    mark_consumed_if_local_ref(self.typed, value, self.flow_ctx);
                }
                if let Some(place) = self.typed.exprs.place_of(expr_id).flatten() {
                    self.flow_ctx.set_place_state(place, VarState::Alive);
                }
                let target_index = super::lower_exprs::target_index_for_expr(self.typed, index);
                if let Some(base_targets) = self.typed.exprs.target_group_of(buf).cloned().flatten()
                {
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
                if super::effects::is_raw_pointer_buffer(self.typed, expr_id)
                    && self.typed.borrows_value(inner)
                {
                    self.expose_raw_address(inner);
                }
                self.validate_pointer_integer_refinement(expr_id, inner);
                self.validate_proven_integer_cast(expr_id, inner);
                if self.typed.exprs.ty_of(inner) != self.typed.exprs.ty_of(expr_id) {
                    self.validate_unpack_authority(inner, expr_id);
                }
                let exposes_borrowed_storage = self.typed.borrows_value(inner)
                    && matches!(self.typed.exprs.ty_of(inner), Some(Ty::OwnedStorage { .. }))
                    && self.typed.exprs.ty_of(expr_id).is_some_and(|ty| {
                        ty.is_ptr()
                            || self
                                .typed
                                .type_registry
                                .integer_validity_for_type(ty)
                                .is_some()
                    });
                if !exposes_borrowed_storage
                    && !matches!(
                        self.typed.exprs.kind[inner.as_usize()],
                        TypedExprKind::Rematerialize { .. }
                    )
                {
                    mark_consumed_if_local_ref(self.typed, inner, self.flow_ctx);
                    if let Some(source) = self.typed.exprs.target_group_of(inner).cloned().flatten()
                    {
                        self.ref_tracker
                            .permanently_invalidate(&source, self.flow_ctx);
                    }
                }
            }
            TypedExprKind::Deref(inner) => {
                self.check_place_use(expr_id, !self.projecting);
                self.walk(inner);
            }
            TypedExprKind::Negate(inner) => {
                self.walk(inner);
            }
            TypedExprKind::SelfRef { .. } => self.check_place_use(expr_id, !self.projecting),
            TypedExprKind::TargetQuery { .. } | TypedExprKind::Target => {}
            TypedExprKind::TargetField(_) => {}
        }
        if let Some(place) = self.typed.temporary_owner(expr_id) {
            self.initialize_temporary_owner(expr_id, place);
        }
        if !is_call {
            self.check_reference_reads(expr_id);
        }
        let value = self.typed.exprs.references_of(idx);
        value.visit(&mut |value| {
            for loan in value.loans.iter().filter(|loan| loan.expression == expr_id) {
                let reference = TrackedReference::Loan(loan.clone());
                self.ref_tracker
                    .targets
                    .entry(reference.clone())
                    .or_default()
                    .extend(&value.targets());
                self.ref_tracker
                    .validity
                    .insert(reference, ReferenceValidity::Valid);
            }
        });
        self.remember_expression_values(expr_id);
        if self.result_exits.contains(&expr_id) {
            self.verify_payload_postconditions(expr_id);
        }
        self.probe_member_completions(expr_id);
    }

    fn check_consuming_argument_aliases(
        &mut self,
        call: ExprId,
        args: &[ExprId],
        conventions: &[ParamConvention],
    ) {
        for (index, arg) in args.iter().enumerate() {
            if !matches!(
                conventions.get(index),
                Some(ParamConvention::Own | ParamConvention::Terminal)
            ) || expr_is_rematerializable(self.typed, *arg)
                || self
                    .typed
                    .exprs
                    .ty_of(*arg)
                    .is_some_and(|ty| is_reference_type(self.typed, ty))
            {
                continue;
            }
            let Some(consumed) = self
                .typed
                .exprs
                .target_group_of(*arg)
                .and_then(Option::as_ref)
            else {
                continue;
            };
            for (other_index, other) in args.iter().enumerate() {
                if index == other_index {
                    continue;
                }
                let mut borrowed = ReferenceTargetSet::default();
                self.typed
                    .exprs
                    .references_of(other.index())
                    .visit(&mut |value| borrowed.extend(&value.targets()));
                if matches!(
                    conventions.get(other_index),
                    Some(ParamConvention::Observe | ParamConvention::Mutate)
                ) && let Some(targets) = self
                    .typed
                    .exprs
                    .target_group_of(*other)
                    .and_then(Option::as_ref)
                {
                    borrowed.extend(targets);
                }
                if !consumed.is_disjoint(&borrowed, &self.flow_ctx.constraint_env) {
                    // type-overlapping-target-group-arguments
                    let diagnostic = Diagnostic::new(
                        TargetGroupCode::OverlappingArguments,
                        "a consuming argument overlaps storage borrowed by another argument",
                    )
                    .at_span_id(self.typed.exprs.span[call.index()], &self.typed.span_table);
                    self.typed.exprs.facts.semantic_facts.flaws[call.index()].push(diagnostic);
                    return;
                }
            }
        }
    }

    fn check_reference_reads(&mut self, expr: ExprId) {
        let reads = &self.typed.exprs.facts.ownership_facts.reference_reads[expr.index()];
        let invalid = reads
            .iter()
            .filter_map(|loan| {
                self.ref_tracker
                    .validity
                    .get(&TrackedReference::Loan(loan.clone()))
            })
            .copied()
            .find(|state| *state != ReferenceValidity::Valid);
        let Some(invalid) = invalid else {
            return;
        };
        let mut already_reported = false;
        let _ = crate::typed::walk_expr_preorder(self.typed, expr, &mut |id| {
            if self.typed.exprs.facts.semantic_facts.flaws[id.index()]
                .iter()
                .any(|flaw| {
                    matches!(
                        flaw.code,
                        DiagnosticCode::Type(TypeCode::Reference(
                            ReferenceCode::UseAfterInvalidation
                        )) | DiagnosticCode::Type(TypeCode::Reference(
                            ReferenceCode::UseAfterPossibleInvalidation
                        ))
                    )
                })
            {
                already_reported = true;
            }
            std::ops::ControlFlow::Continue(())
        });
        if already_reported {
            return;
        }
        let (code, message) = if invalid == ReferenceValidity::MaybeInvalid {
            (
                DiagnosticCode::Type(TypeCode::Reference(
                    // type-use-of-maybe-invalidated-reference
                    ReferenceCode::UseAfterPossibleInvalidation,
                )),
                "borrowed contents may have been invalidated on another control-flow path",
            )
        } else {
            (
                DiagnosticCode::Type(TypeCode::Reference(ReferenceCode::UseAfterInvalidation)),
                "cannot access invalidated borrowed contents",
            )
        };
        let flaw = Diagnostic::new(code, message)
            .at_span_id(self.typed.exprs.span[expr.index()], &self.typed.span_table);
        self.typed.exprs.facts.semantic_facts.flaws[expr.index()].push(flaw);
    }

    fn check_tag_refinements(
        &mut self,
        expr: ExprId,
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
        let mut substitution = crate::subst::DepSubst::new();
        if let Some(instance) = self.typed.exprs.ty[expr.as_usize()].named_instance() {
            for (name, argument) in &instance.arguments {
                match argument {
                    ast::TyArg::Type(ty) => {
                        substitution.types.insert(*name, ty.as_ref().clone());
                    }
                    ast::TyArg::Const(value) => {
                        substitution.consts.insert(*name, value.clone());
                    }
                }
            }
        }
        let ordered_fields: Vec<Intern<String>> = match self
            .typed
            .type_registry
            .resolved_definition_for_type(&tag.resolved_ty)
        {
            crate::ty::Ty::Record { fields, .. } => fields.iter().map(|(n, _)| *n).collect(),
            _ => return,
        };
        // Build a map of all field values so refinements can reference sibling fields.
        let all_field_values: HashMap<Intern<String>, NormalExpr> = {
            let mut m = HashMap::new();
            for (j, arg_id) in arg_ids.iter().enumerate() {
                let fname = field_names
                    .get(j)
                    .filter(|n| ordered_fields.contains(n))
                    .copied()
                    .or_else(|| ordered_fields.get(j).copied());
                if let Some(fname) = fname {
                    let value = super::symbolic_values::normal_expr(
                        self.typed,
                        Some(self.callable_signatures),
                        self.local_const_values,
                        *arg_id,
                    )
                    .unwrap_or_else(|| NormalExpr::Var(Intern::new(format!("$value{}", arg_id.0))));
                    m.insert(fname, value);
                }
            }
            m
        };
        let span_table = &self.typed.span_table;
        for (i, arg_id) in arg_ids.iter().enumerate() {
            let field_name = field_names
                .get(i)
                .filter(|n| ordered_fields.contains(n))
                .copied()
                .or_else(|| ordered_fields.get(i).copied());
            let Some(field_name) = field_name else {
                continue;
            };
            let Some(refinement) = refinements.get(&field_name) else {
                continue;
            };
            let Some(field_value) = all_field_values.get(&field_name).cloned() else {
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
            let field_types: HashMap<_, _> = arg_ids
                .iter()
                .enumerate()
                .filter_map(|(j, arg)| {
                    let name = field_names
                        .get(j)
                        .filter(|n| ordered_fields.contains(n))
                        .or_else(|| ordered_fields.get(j))?;
                    Some((*name, self.typed.exprs.ty[arg.index()].clone()))
                })
                .collect();
            super::refinement_terms::extend_values(
                self.typed,
                self.callable_signatures,
                &field_types,
                &mut var_values,
                refinement,
            );
            let predicate = predicate_expr_to_predicate(refinement, field_value, &var_values);
            let predicate =
                resolve_target_queries_in_predicate(&predicate, &substitution, self.typed);
            let mut constraints = self.flow_ctx.constraint_env.clone();
            for (j, arg) in arg_ids.iter().enumerate() {
                if let Some(name) = field_names
                    .get(j)
                    .filter(|name| ordered_fields.contains(name))
                    .or_else(|| ordered_fields.get(j))
                    && let Some(value) = all_field_values.get(name)
                {
                    record_constraints::assume_integer_bounds(
                        self.typed,
                        &self.typed.exprs.ty[arg.index()],
                        value.clone(),
                        &mut constraints,
                    );
                }
            }
            match constraints.prove(&predicate, &var_values) {
                ProveResult::Proven => {}
                ProveResult::Disproven => {
                    let Some(span) = self.typed.exprs.span_of(*arg_id) else {
                        continue;
                    };
                    self.typed.exprs.facts.semantic_facts.flaws[arg_id.index()].push(
                        // dep-refinement-failed
                        Diagnostic::new(
                            ProofCode::DependentRefinementFailed,
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
                    let Some(span) = self.typed.exprs.span_of(*arg_id) else {
                        continue;
                    };
                    self.typed.exprs.facts.semantic_facts.flaws[arg_id.index()].push(
                        refinement_diagnostics::construction(field_name.as_str(), refinement)
                            .at_span_id(span, span_table),
                    );
                }
            }
        }
    }
}

fn destructure_remainder_is_discardable(
    value_ty: &Ty,
    tag_name: Intern<String>,
    bound_fields: &HashSet<Intern<String>>,
) -> bool {
    match value_ty {
        Ty::Record { fields, .. } => fields.iter().all(|(name, field_ty)| {
            bound_fields.contains(name) || field_ty.is_structurally_discardable()
        }),
        Ty::Union { variants, .. } => variants
            .iter()
            .find(|variant| variant.name == tag_name)
            .is_none_or(|variant| {
                variant.fields.iter().all(|(name, field_ty)| {
                    bound_fields.contains(name) || field_ty.is_structurally_discardable()
                })
            }),
        _ => true,
    }
}

pub(super) fn expr_to_normal_expr(
    kinds: &[TypedExprKind],
    const_values: &[Option<ConstValue>],
    lcv: &HashMap<Intern<String>, ConstValue>,
    expr_id: ExprId,
) -> Option<NormalExpr> {
    let idx = expr_id.index();
    if let Some(cv) = const_values.get(idx).and_then(Clone::clone) {
        return Some(NormalExpr::Value(cv));
    }
    match kinds.get(idx)? {
        TypedExprKind::Lit(lit) => match lit {
            Literal::Int(n) => Some(NormalExpr::Value(ConstValue::Int(*n))),
            _ => None,
        },
        TypedExprKind::Negate(inner) => expr_to_normal_expr(kinds, const_values, lcv, *inner)
            .and_then(|value| match value {
                NormalExpr::Value(ConstValue::Int(value)) => {
                    Some(NormalExpr::Value(ConstValue::Int(-value)))
                }
                _ => None,
            }),
        TypedExprKind::Ref(inner) | TypedExprKind::TakePtr(inner) | TypedExprKind::Deref(inner) => {
            expr_to_normal_expr(kinds, const_values, lcv, *inner)
        }
        TypedExprKind::Cast { expr, .. } => expr_to_normal_expr(kinds, const_values, lcv, *expr),
        TypedExprKind::Rematerialize { source, .. } => {
            expr_to_normal_expr(kinds, const_values, lcv, *source)
        }
        TypedExprKind::TargetQuery { kind, operand } => Some(NormalExpr::TargetQuery {
            kind: *kind,
            operand: crate::subst::type_reference_for_ty(operand)?,
        }),
        TypedExprKind::FnCall { target, args, .. } if args.is_none() => lcv
            .get(&target.0)
            .cloned()
            .map(NormalExpr::Value)
            .or(Some(NormalExpr::Var(target.0))),
        TypedExprKind::SelfRef { .. } => Some(NormalExpr::Var(Intern::from_ref("self"))),
        TypedExprKind::AggregateGet { base, index } => {
            let NormalExpr::Var(base) = expr_to_normal_expr(kinds, const_values, lcv, *base)?
            else {
                return None;
            };
            Some(super::symbolic_values::field_symbol(base, *index))
        }
        TypedExprKind::IntrinsicCall { op, args } if args.len() == 2 => {
            let left = expr_to_normal_expr(kinds, const_values, lcv, args[0])?;
            let right = expr_to_normal_expr(kinds, const_values, lcv, args[1])?;
            match op {
                crate::typed::IntrinsicOp::BitsAdd => {
                    Some(NormalExpr::Add(Box::new(left), Box::new(right)))
                }
                crate::typed::IntrinsicOp::BitsSubtract => {
                    Some(NormalExpr::Sub(Box::new(left), Box::new(right)))
                }
                crate::typed::IntrinsicOp::BitsMultiply => {
                    Some(NormalExpr::Mul(Box::new(left), Box::new(right)))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn rebind_result_within_integer_bounds(
    typed: &TypedFileAst,
    value: ExprId,
    min: i128,
    max: i128,
) -> bool {
    if let Some(Some(ConstValue::Int(value))) = typed.exprs.const_value_of(value) {
        return I256::from(min) <= *value && *value <= I256::from(max);
    }
    match typed.exprs.kind_of(value) {
        Some(TypedExprKind::FnCall { .. } | TypedExprKind::Cast { .. }) => {
            typed.exprs.ty_of(value).is_some_and(|ty| {
                typed
                    .type_registry
                    .integer_validity_for_type(ty)
                    .or_else(|| ty.anonymous_integer_validity())
                    .and_then(|validity| validity.domain().storage_hull())
                    .is_some_and(|hull| {
                        I256::from(min) <= hull.min() && hull.max() <= I256::from(max)
                    })
            })
        }
        Some(TypedExprKind::When(branches)) => branches.arms.iter().all(|arm| {
            let body = match arm {
                TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. } => *body,
                TypedWhenArm::Else(body, _) => *body,
            };
            rebind_result_within_integer_bounds(typed, body, min, max)
        }),
        _ => false,
    }
}

fn rebind_rhs_within_integer_bounds(
    rhs_expr: NormalExpr,
    local_value: NormalExpr,
    lcv: &HashMap<Intern<String>, ConstValue>,
    min: i128,
    max: i128,
    divides_upper_bound: bool,
    constraints: &ConstraintEnv,
) -> bool {
    if let NormalExpr::Value(ConstValue::Int(value)) = &rhs_expr {
        return I256::from(min) <= *value && *value <= I256::from(max);
    }
    let known_vars = lcv
        .iter()
        .filter_map(|(name, value)| value.as_const_size_int().map(|value| (*name, value)))
        .map(|(name, value)| (name, NormalExpr::Value(ConstValue::Int(value.into()))))
        .collect::<HashMap<_, _>>();
    let lower_bound_proven = constraints
        .prove(&Predicate::Ge(rhs_expr.clone(), min.into()), &known_vars)
        == ProveResult::Proven
        || matches!(&rhs_expr,
            NormalExpr::Mul(lhs, rhs)
                if **lhs == local_value
                    && rhs.as_const_int() == Some(2)
                    && constraints.prove(
                        &Predicate::Ge(local_value.clone(), (min / 2 + i128::from(min % 2 > 0)).into()),
                        &known_vars,
                    ) == ProveResult::Proven
        );
    if !lower_bound_proven {
        return false;
    }
    if matches!(
        constraints.prove(
            &Predicate::Le(
                rhs_expr.clone(),
                NormalExpr::Value(ConstValue::Int(max.into())),
            ),
            &known_vars
        ),
        ProveResult::Proven,
    ) {
        return true;
    }

    match rhs_expr {
        NormalExpr::Add(lhs, rhs) if *lhs == local_value => {
            let Some(delta) = rhs.as_const_int() else {
                return false;
            };
            constraints.prove(
                &Predicate::Le(
                    *lhs,
                    NormalExpr::Value(ConstValue::Int(I256::from(max) - I256::from(delta))),
                ),
                &known_vars,
            ) == ProveResult::Proven
        }
        NormalExpr::Mul(lhs, rhs)
            if divides_upper_bound && *lhs == local_value && rhs.as_const_int() == Some(2) =>
        {
            constraints.prove(
                &Predicate::Lt(*lhs, NormalExpr::Value(ConstValue::Int(max.into()))),
                &known_vars,
            ) == ProveResult::Proven
        }
        _ => false,
    }
}

fn refinement_divides_storage_max(domain: &ast::integer::IntegerDomain) -> bool {
    let Some(hull) = domain.storage_hull() else {
        return false;
    };
    if hull.min() <= I256::from(0) {
        return false;
    }
    let ast::integer::CanonicalPredicate::Structured(predicate) = domain.predicate() else {
        return false;
    };
    proposition_divides_value(predicate, hull.max())
}

fn proposition_divides_value(predicate: &ast::PredicateExpr, dividend: I256) -> bool {
    match predicate {
        ast::PredicateExpr::And(predicates) => predicates
            .iter()
            .any(|predicate| proposition_divides_value(predicate, dividend)),
        ast::PredicateExpr::Proposition(proposition) => matches!(
            proposition.as_ref(),
            ast::ProofProposition::Compare {
                left,
                relation: ast::ProofRelation::Equal,
                right: ast::ProofTerm::Value(zero),
            } if *zero == I256::from(0)
                && matches!(left,
                    ast::ProofTerm::Remainder(left, right)
                        if matches!(left.as_ref(), ast::ProofTerm::Value(value) if *value == dividend)
                            && matches!(right.as_ref(), ast::ProofTerm::Name(name) if name.as_str() == "self")
                )
        ),
        _ => false,
    }
}

fn comparison_facts(
    left: NormalExpr,
    relation: ast::ProofRelation,
    right: NormalExpr,
) -> Vec<(NormalExpr, NormalExpr, bool)> {
    match relation {
        ast::ProofRelation::Less => vec![(left, right, true)],
        ast::ProofRelation::LessOrEqual => vec![(left, right, false)],
        ast::ProofRelation::Greater => vec![(right, left, true)],
        ast::ProofRelation::GreaterOrEqual => vec![(right, left, false)],
        ast::ProofRelation::Equal => {
            vec![(left.clone(), right.clone(), false), (right, left, false)]
        }
        ast::ProofRelation::NotEqual => Vec::new(),
    }
}

fn condition_comparison_facts(
    condition: &crate::typed::TypedCondition,
    taken: bool,
    typed: &TypedFileAst,
    signatures: Option<&CallableSignatureScope<'_>>,
    local_values: &HashMap<Intern<String>, ConstValue>,
    frozen: Option<&HashMap<ExprId, Vec<crate::analysis::ScalarComparison>>>,
    frozen_scalars: Option<&HashMap<ExprId, NormalExpr>>,
) -> Vec<(NormalExpr, NormalExpr, bool)> {
    match condition {
        crate::typed::TypedCondition::Proposition { subject } => {
            if let Some(frozen) = frozen {
                let Some(value) = frozen.get(subject).and_then(|values| {
                    values.iter().find(|value| {
                        value.outcome == crate::analysis::ComparisonOutcome::Proposition(taken)
                    })
                }) else {
                    return Vec::new();
                };
                return comparison_facts(
                    value.left.clone(),
                    value.comparison.into(),
                    value.right.clone(),
                );
            }
            let Some(crate::typed::EvidenceProposition::IntegerComparison {
                lhs,
                rhs,
                comparison,
            }) = typed.exprs.proposition_evidence_of(*subject)
            else {
                return Vec::new();
            };
            let (Some(left), Some(right)) = (
                super::symbolic_values::normal_expr(typed, signatures, local_values, *lhs),
                super::symbolic_values::normal_expr(typed, signatures, local_values, *rhs),
            ) else {
                return Vec::new();
            };
            let relation = ast::ProofRelation::from(*comparison);
            comparison_facts(
                left,
                if taken {
                    relation
                } else {
                    relation.complement()
                },
                right,
            )
        }
        crate::typed::TypedCondition::Compare {
            left,
            relation,
            right,
        } => {
            let scalar = |expression: ExprId| match frozen_scalars {
                Some(values) => values.get(&expression).cloned(),
                None => {
                    super::symbolic_values::normal_expr(typed, signatures, local_values, expression)
                }
            };
            let (Some(left), Some(right)) = (scalar(*left), scalar(*right)) else {
                return Vec::new();
            };
            let relation = if taken {
                *relation
            } else {
                relation.complement()
            };
            comparison_facts(left, relation, right)
        }
        crate::typed::TypedCondition::Is { subject, pattern } => {
            if taken
                && let ast::Pattern::InRange {
                    bounds: ast::InRangeBounds::Literal(lower, upper),
                    ..
                } = &pattern.value
                && let Some(value) = match frozen_scalars {
                    Some(values) => values.get(subject).cloned(),
                    None => super::symbolic_values::normal_expr(
                        typed,
                        signatures,
                        local_values,
                        *subject,
                    ),
                }
            {
                return vec![
                    ((*lower).into(), value.clone(), false),
                    (value, (*upper).into(), false),
                ];
            }
            let ast::Pattern::Nominal(label, _) = &pattern.value else {
                return Vec::new();
            };
            let Some(evidence) = typed.exprs.result_evidence_of(*subject) else {
                return Vec::new();
            };
            let selected = if taken {
                *label
            } else if evidence.alternatives.len() == 2
                && evidence
                    .alternatives
                    .iter()
                    .any(|alternative| alternative.label == *label)
            {
                evidence
                    .alternatives
                    .iter()
                    .find(|alternative| alternative.label != *label)
                    .unwrap()
                    .label
            } else {
                return Vec::new();
            };
            let Some(alternative) = evidence
                .alternatives
                .iter()
                .find(|alternative| alternative.label == selected)
            else {
                return Vec::new();
            };
            let (lhs, rhs, comparison) = if let Some(frozen) = frozen {
                let Some(value) = frozen.get(subject).and_then(|values| {
                    values.iter().find(|value| {
                        value.outcome == crate::analysis::ComparisonOutcome::Alternative(selected)
                    })
                }) else {
                    return Vec::new();
                };
                (value.left.clone(), value.right.clone(), value.comparison)
            } else {
                let crate::typed::EvidenceProposition::IntegerComparison {
                    lhs,
                    rhs,
                    comparison,
                } = alternative.proposition;
                let Some(lhs) =
                    super::symbolic_values::normal_expr(typed, signatures, local_values, lhs)
                else {
                    return Vec::new();
                };
                let Some(rhs) =
                    super::symbolic_values::normal_expr(typed, signatures, local_values, rhs)
                else {
                    return Vec::new();
                };
                (lhs, rhs, comparison)
            };
            comparison_facts(lhs, comparison.into(), rhs)
        }

        crate::typed::TypedCondition::Not(inner) => condition_comparison_facts(
            inner,
            !taken,
            typed,
            signatures,
            local_values,
            frozen,
            frozen_scalars,
        ),
        crate::typed::TypedCondition::And(left, right) if taken => {
            let mut facts = condition_comparison_facts(
                left,
                true,
                typed,
                signatures,
                local_values,
                frozen,
                frozen_scalars,
            );
            facts.extend(condition_comparison_facts(
                right,
                true,
                typed,
                signatures,
                local_values,
                frozen,
                frozen_scalars,
            ));
            facts
        }
        crate::typed::TypedCondition::Or(left, right) if !taken => {
            let mut facts = condition_comparison_facts(
                left,
                false,
                typed,
                signatures,
                local_values,
                frozen,
                frozen_scalars,
            );
            facts.extend(condition_comparison_facts(
                right,
                false,
                typed,
                signatures,
                local_values,
                frozen,
                frozen_scalars,
            ));
            facts
        }
        crate::typed::TypedCondition::And(_, _) | crate::typed::TypedCondition::Or(_, _) => {
            Vec::new()
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ConditionPattern<'a> {
    subject: ExprId,
    pattern: &'a ast::Pattern,
    taken: bool,
    stable: bool,
}

pub(super) fn condition_pattern_paths(
    condition: &crate::typed::TypedCondition,
    taken: bool,
) -> Vec<Vec<ConditionPattern<'_>>> {
    match condition {
        crate::typed::TypedCondition::Proposition { .. }
        | crate::typed::TypedCondition::Compare { .. } => vec![Vec::new()],
        crate::typed::TypedCondition::Is { subject, pattern } => vec![vec![ConditionPattern {
            subject: *subject,
            pattern: &pattern.value,
            taken,
            stable: true,
        }]],
        crate::typed::TypedCondition::Not(inner) => condition_pattern_paths(inner, !taken),
        crate::typed::TypedCondition::And(left, right) if taken => combine_pattern_paths(
            condition_pattern_paths(left, true),
            condition_pattern_paths(right, true),
        ),
        crate::typed::TypedCondition::And(left, right) => {
            let mut paths = condition_pattern_paths(left, false);
            paths.extend(combine_pattern_paths(
                condition_pattern_paths(left, true),
                condition_pattern_paths(right, false),
            ));
            paths
        }
        crate::typed::TypedCondition::Or(left, right) if taken => {
            let mut paths = condition_pattern_paths(left, true);
            paths.extend(combine_pattern_paths(
                condition_pattern_paths(left, false),
                condition_pattern_paths(right, true),
            ));
            paths
        }
        crate::typed::TypedCondition::Or(left, right) => combine_pattern_paths(
            condition_pattern_paths(left, false),
            condition_pattern_paths(right, false),
        ),
    }
}

fn combine_pattern_paths<'a>(
    left: Vec<Vec<ConditionPattern<'a>>>,
    right: Vec<Vec<ConditionPattern<'a>>>,
) -> Vec<Vec<ConditionPattern<'a>>> {
    left.into_iter()
        .flat_map(|left_path| {
            right.iter().map(move |right_path| {
                let mut combined = left_path.clone();
                combined.extend(right_path);
                combined
            })
        })
        .collect()
}

pub(super) fn pattern_moves_subject(pattern: &ast::Pattern) -> bool {
    match pattern {
        ast::Pattern::Nominal(name, _) => {
            name.as_str() != "_" && name.as_str().chars().next().is_some_and(char::is_lowercase)
        }
        ast::Pattern::Generic { params, .. } => params.iter().any(|(name, kind)| {
            matches!(kind, ast::ParameterKind::Generic)
                && name.as_str() != "_"
                && name.as_str().chars().next().is_some_and(char::is_lowercase)
        }),
        ast::Pattern::Pointer(inner) => pattern_moves_subject(&inner.value),
        ast::Pattern::ListCons { head, tail } => {
            pattern_moves_subject(&head.value) || pattern_moves_subject(&tail.value)
        }
        ast::Pattern::Invalid
        | ast::Pattern::Ref { .. }
        | ast::Pattern::Qualified(_)
        | ast::Pattern::Literal(_, _)
        | ast::Pattern::Unit
        | ast::Pattern::ListEmpty
        | ast::Pattern::InRange { .. } => false,
    }
}

fn pattern_moves_empty_variant(
    typed: &TypedFileAst,
    pattern: &ast::Pattern,
    subject: ExprId,
) -> bool {
    let ast::Pattern::Nominal(name, _) = pattern else {
        return false;
    };
    if typed.borrows_value(subject) {
        return false;
    }
    let Some(ty) = typed.exprs.ty_of(subject) else {
        return false;
    };
    let Ty::Union { variants, .. } = typed.type_registry.resolved_definition_for_type(ty) else {
        return false;
    };
    variants.iter().any(|variant| !variant.fields.is_empty())
        && variants
            .iter()
            .any(|variant| variant.name == *name && variant.fields.is_empty())
}

#[cfg(test)]
mod comparison_evidence_tests {
    use super::*;
    use crate::transform::{TransformCtx, transform};
    use crate::{BindBody, DefId, FileId};
    use parser::cursor::TokenCursor;

    #[test]
    fn anonymous_structural_destructure_remainder_is_discardable() {
        let head = Intern::from_ref("head");
        let tail = Intern::from_ref("tail");
        let ty = Ty::Record {
            name: Intern::from_ref(""),
            fields: vec![
                (head, Box::new(Ty::anonymous_integer_for_width(8, false))),
                (
                    tail,
                    Box::new(Ty::Record {
                        name: Intern::from_ref(""),
                        fields: vec![
                            (
                                Intern::from_ref("0"),
                                Box::new(Ty::anonymous_integer_for_width(8, false)),
                            ),
                            (Intern::from_ref("1"), Box::new(Ty::Unit)),
                        ],
                        resolved_params: None,
                    }),
                ),
            ],
            resolved_params: None,
        };

        assert!(destructure_remainder_is_discardable(
            &ty,
            Intern::from_ref(""),
            &HashSet::from([head]),
        ));
    }

    #[test]
    fn true_and_false_alternatives_supply_complementary_flow_facts() {
        let parsed = TokenCursor::parse_source(
            "main:\n    when 1 < 2 then\n        1\n        return 0\n    return 0\n",
        );
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        let body = match &typed.defs[&DefId::named(Intern::from_ref("main"))].body {
            BindBody::Body { exprs, .. } => exprs[0],
            _ => panic!("expected body"),
        };
        let TypedExprKind::When(branch) = &typed.exprs.kind[body.as_usize()] else {
            panic!("when expression")
        };
        let TypedWhenArm::Cond { condition, .. } = &branch.arms[0] else {
            panic!("condition arm")
        };

        let taken =
            condition_comparison_facts(condition, true, &typed, None, &HashMap::new(), None, None);
        let rejected =
            condition_comparison_facts(condition, false, &typed, None, &HashMap::new(), None, None);

        assert_eq!(
            taken,
            vec![(
                NormalExpr::Value(ConstValue::Int(1.into())),
                NormalExpr::Value(ConstValue::Int(2.into())),
                true,
            )]
        );
        assert_eq!(
            rejected,
            vec![(
                NormalExpr::Value(ConstValue::Int(2.into())),
                NormalExpr::Value(ConstValue::Int(1.into())),
                false,
            )]
        );
    }

    fn condition_subject(id: u32, name: &str) -> crate::typed::TypedCondition {
        crate::typed::TypedCondition::Is {
            subject: ExprId(id),
            pattern: Box::new(ast::Spanned {
                value: ast::Pattern::Nominal(Intern::new(name.to_string()), ast::SpanId::INVALID),
                span_id: ast::SpanId::INVALID,
            }),
        }
    }

    fn condition_ownership_paths(
        condition: &crate::typed::TypedCondition,
        taken: bool,
    ) -> Vec<HashSet<ExprId>> {
        condition_pattern_paths(condition, taken)
            .into_iter()
            .map(|path| {
                path.into_iter()
                    .filter(|fact| fact.taken && pattern_moves_subject(fact.pattern))
                    .map(|fact| fact.subject)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn condition_ownership_paths_model_not_and_and_or_joins() {
        let left = condition_subject(1, "left_value");
        let right = condition_subject(2, "right_value");
        let observed = condition_subject(3, "True");

        assert_eq!(condition_ownership_paths(&observed, true), [HashSet::new()]);
        assert_eq!(
            condition_ownership_paths(
                &crate::typed::TypedCondition::Not(Box::new(left.clone())),
                false,
            ),
            [HashSet::from([ExprId(1)])]
        );
        assert_eq!(
            condition_ownership_paths(
                &crate::typed::TypedCondition::And(Box::new(left.clone()), Box::new(right.clone()),),
                false,
            ),
            [HashSet::new(), HashSet::from([ExprId(1)])]
        );
        assert_eq!(
            condition_ownership_paths(
                &crate::typed::TypedCondition::Or(Box::new(left), Box::new(right)),
                true,
            ),
            [HashSet::from([ExprId(1)]), HashSet::from([ExprId(2)])]
        );
    }

    #[test]
    fn terminating_condition_move_does_not_leak_into_rejected_continuation() {
        let parsed = TokenCursor::parse_source(
            "Int is in 0 to 255\nChoice is Some(Int) or None\nkeep(value Choice) Choice:\n    when value is Some(payload) then\n        Choice.None\n        return\n    return value",
        );
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        let codes = typed
            .all_flaws()
            .into_iter()
            .map(|(_, flaw)| flaw.code.to_string())
            .collect::<Vec<_>>();

        assert!(
            !codes.iter().any(|code| code.contains("moved-value")),
            "{codes:?}"
        );
    }

    #[test]
    fn successful_by_value_condition_binding_moves_subject_inside_branch() {
        let parsed = TokenCursor::parse_source(
            "Int is in 0 to 255\nChoice is Some(Int) or None\nbad(value Choice) Choice:\n    when value is Some(payload) then\n        value\n        return\n    return value",
        );
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        let codes = typed
            .all_flaws()
            .into_iter()
            .map(|(_, flaw)| flaw.code.to_string())
            .collect::<Vec<_>>();

        assert!(
            codes
                .iter()
                .any(|code| code == "type-use-after-move" || code == "type-use-of-moved-value"),
            "{codes:?}"
        );
    }
}

#[cfg(test)]
#[path = "flow/binding_tests.rs"]
mod binding_tests;

#[cfg(test)]
#[path = "flow/rebind_bounds_tests.rs"]
mod rebind_bounds_tests;
