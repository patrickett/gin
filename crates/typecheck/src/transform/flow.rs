//! Stage 3: Flow — Compute flow contexts, attach flow flaws.
//!
//! This stage walks the typed expression arena depth-first, computes
//! `FlowContext` at each program point, and attaches flow-related flaws
//! (ownership, bounds, narrowing, lin-value checks, etc.).

use std::collections::{HashMap, HashSet};

use crate::analysis::{FlowContext, TyOwnershipExt, VarState, expr_is_rematerializable};
use crate::solver::{ConstraintEnv, Predicate, ProveResult, predicate_expr_to_predicate};
use ast::expr::Literal;
use ast::normal_expr::NormalExpr;
use ast::{ConstValue, ParamConvention};
use diagnostic::{Diagnostic, RelatedSpan};

use crate::ty::{Ty, reference_pointee_for_type, reference_semantics_for_type};
use crate::typed::{
    DefId, ExprId, GroupId, Overlap, PlaceId, ReferenceTargetGroup, ReferenceTargetSet, TagId,
    TargetIndex, TypedCallableSignature, TypedExprKind, TypedFileAst, TypedLoopKind, TypedWhenArm,
    VariantId,
};
use i256::I256;
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
    targets: HashMap<PlaceId, ReferenceTargetSet>,
    validity: HashMap<PlaceId, ReferenceValidity>,
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
            self.targets.insert(reference, target);
            self.validity.insert(reference, ReferenceValidity::Valid);
        }
    }

    fn replace(&mut self, place: &ReferenceTargetGroup, flow_ctx: &mut FlowContext) {
        self.permanently_invalidate(&ReferenceTargetSet::singleton(place.clone()), flow_ctx);
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
                flow_ctx.set_place_state(*reference, VarState::Invalidated);
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
                    target.is_strict_descendant_of(source, &flow_ctx.constraint_env)
                        || source.is_strict_descendant_of(target, &flow_ctx.constraint_env)
                        || target.overlap(source, &flow_ctx.constraint_env) != Overlap::Disjoint
                })
            }) {
                self.validity
                    .insert(*reference, ReferenceValidity::PermanentlyInvalid);
                flow_ctx.set_place_state(*reference, VarState::Invalidated);
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
        let owned_params: Vec<(Intern<String>, Ty, PlaceId)> = typed
            .defs
            .get(def_id)
            .map(|bind| {
                bind.params
                    .iter()
                    .zip(&bind.param_conventions)
                    .enumerate()
                    .filter_map(|(slot, ((name, ty), convention))| {
                        if *convention != ParamConvention::Own || is_reference_type(typed, ty) {
                            return None;
                        }
                        let binder = ast::BinderId::new(
                            typed.file_id.0,
                            ast::BinderOwner::Parameter {
                                definition: def_id.0,
                                slot: slot as u32,
                            },
                        );
                        typed
                            .places
                            .iter()
                            .position(|place| place.binder == binder)
                            .map(|place| (*name, ty.clone(), PlaceId(place as u32)))
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
        for (_, _, place) in &owned_params {
            flow_ctx.set_place_state(*place, VarState::Alive);
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
            .filter_map(|(name, ty, place)| {
                (!matches!(ctx.flow_ctx.place_state(*place), Some(VarState::Consumed))
                    && !ty.is_structurally_discardable())
                .then_some(*name)
            })
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
                bind.flaws.push(
                    Diagnostic::new(
                        "type-owned-value-not-consumed",
                        format!("owned value `{}` is not returned or consumed", name),
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
            } if typed.exprs.place[idx].is_some_and(|place| {
                matches!(flow_ctx.place_state(place), Some(VarState::Declared))
            }) =>
            {
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
    let ty = typed.exprs.ty_of(base)?;
    let ty =
        reference_pointee_for_type(ty, Some(&typed.type_registry)).unwrap_or_else(|| ty.clone());
    let Ty::Record { fields, .. } = typed.type_registry.resolved_definition_for_type(&ty) else {
        return None;
    };
    fields.iter().position(|(name, _)| *name == field)
}

fn validate_call_refinements(
    typed: &TypedFileAst,
    args: &[ExprId],
    signature: &TypedCallableSignature,
    substituted_return: Option<&Ty>,
    constraints: &ConstraintEnv,
) -> Vec<Diagnostic> {
    let mut substitutions = HashMap::new();
    let mut type_substitutions = HashMap::new();
    if let Some(substituted_return) = substituted_return {
        collect_dependent_type_substitutions(
            &signature.return_type,
            substituted_return,
            &mut type_substitutions,
        );
    }
    for ((name, formal_ty), arg) in signature.params.iter().zip(args) {
        if let TargetIndex::Symbolic(actual) = target_index_for_expr(typed, *arg) {
            substitutions.insert(*name, actual);
        }
        if let Some(actual_ty) = typed.exprs.ty_of(*arg) {
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
        match constraints.prove(&resolved_predicate, &HashMap::new()) {
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
            ProveResult::Unknown if proven_by_target => {}
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

fn resolve_target_queries_in_predicate(
    predicate: &crate::solver::Predicate,
    substitution: &crate::subst::DepSubst,
    typed: &TypedFileAst,
) -> crate::solver::Predicate {
    use crate::solver::Predicate;

    let resolve = |expr: &NormalExpr| {
        resolve_target_queries_in_normal(&substitution.apply_to_normal(expr), typed)
    };
    match predicate {
        Predicate::Lt(left, right) => Predicate::Lt(resolve(left), resolve(right)),
        Predicate::Gt(left, right) => Predicate::Gt(resolve(left), resolve(right)),
        Predicate::Le(left, right) => Predicate::Le(resolve(left), resolve(right)),
        Predicate::Ge(left, right) => Predicate::Ge(resolve(left), resolve(right)),
        Predicate::Eq(left, right) => Predicate::Eq(resolve(left), resolve(right)),
        Predicate::Ne(left, right) => Predicate::Ne(resolve(left), resolve(right)),
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

fn resolve_target_queries_in_normal(expr: &NormalExpr, typed: &TypedFileAst) -> NormalExpr {
    match expr {
        NormalExpr::Add(left, right) => NormalExpr::Add(
            Box::new(resolve_target_queries_in_normal(left, typed)),
            Box::new(resolve_target_queries_in_normal(right, typed)),
        ),
        NormalExpr::Sub(left, right) => NormalExpr::Sub(
            Box::new(resolve_target_queries_in_normal(left, typed)),
            Box::new(resolve_target_queries_in_normal(right, typed)),
        ),
        NormalExpr::Mul(left, right) => NormalExpr::Mul(
            Box::new(resolve_target_queries_in_normal(left, typed)),
            Box::new(resolve_target_queries_in_normal(right, typed)),
        ),
        NormalExpr::TargetQuery { kind, operand } => typed
            .target_layout
            .and_then(|layout| {
                let name = operand.head_name()?;
                let ty = typed.tag_types.get(&TagId(name))?;
                layout.query(*kind, ty, Some(&typed.type_registry)).ok()
            })
            .map(|value| NormalExpr::Value(ConstValue::Int(value.into())))
            .unwrap_or_else(|| expr.clone()),
        NormalExpr::Value(_) | NormalExpr::Var(_) | NormalExpr::Inferred(_) => expr.clone(),
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

fn mark_consumed_if_local_ref(typed: &TypedFileAst, expr_id: ExprId, flow_ctx: &mut FlowContext) {
    let Some(place) = typed.exprs.place.get(expr_id.as_usize()).copied().flatten() else {
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
        TypedExprKind::ConsumeArg(inner) | TypedExprKind::Eat(inner) => {
            expr_consumes_target(typed, *inner, target_name)
        }
        TypedExprKind::Ref(inner)
        | TypedExprKind::TakePtr(inner)
        | TypedExprKind::Deref(inner)
        | TypedExprKind::Negate(inner)
        | TypedExprKind::Cast { expr: inner, .. } => {
            expr_consumes_target(typed, *inner, target_name)
        }
        TypedExprKind::TupleGet { base, .. } | TypedExprKind::BufGet { buf: base, .. } => {
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
        | TypedExprKind::TupleGet { base: inner, .. }
        | TypedExprKind::BufGet { buf: inner, .. }
        | TypedExprKind::Negate(inner)
        | TypedExprKind::Cast { expr: inner, .. } => {
            expr_consumes_target(typed, *inner, target_name)
        }
        TypedExprKind::FnCall { target, .. } if target.0 == target_name => true,
        TypedExprKind::ConsumeArg(inner) | TypedExprKind::Eat(inner) => {
            expr_consumes_target(typed, *inner, target_name)
        }
        _ => false,
    }
}

fn expression_consumes_place(
    typed: &TypedFileAst,
    expression: ExprId,
    place: crate::typed::PlaceId,
) -> bool {
    let Some(kind) = typed.exprs.kind_of(expression) else {
        return false;
    };
    if let TypedExprKind::ConsumeArg(inner) | TypedExprKind::Eat(inner) = kind
        && typed.exprs.place[inner.as_usize()] == Some(place)
    {
        return true;
    }
    let mut consumes = false;
    let _ = crate::typed::walk_expr_children(kind, &mut |child| {
        if expression_consumes_place(typed, child, place) {
            consumes = true;
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    });
    consumes
}

fn is_reference_type(typed: &TypedFileAst, ty: &Ty) -> bool {
    reference_semantics_for_type(ty, Some(&typed.type_registry)).is_some()
}

struct WalkExprCtx<'a> {
    typed: &'a mut TypedFileAst,
    flow_ctx: &'a mut FlowContext,
    local_var_types: &'a HashMap<Intern<String>, Ty>,
    ref_tracker: &'a mut RefTracker,
    const_bindings: &'a mut HashMap<Intern<String>, bool>,

    local_const_values: &'a mut HashMap<Intern<String>, ConstValue>,
    callable_signatures: &'a HashMap<DefId, TypedCallableSignature>,
}

impl<'a> WalkExprCtx<'a> {
    fn validate_projected_write(&mut self, expression: ExprId, value: ExprId) {
        let index = expression.as_usize();
        let Some(place_id) = self.typed.exprs.place[index] else {
            return;
        };
        let place = &self.typed.places[place_id.0 as usize];
        if !place.mutable {
            self.typed.exprs.flaws[index].push(
                Diagnostic::new(
                    "type-rebind-immutable",
                    format!("cannot rebind immutable value `{}`", place.name.as_str()),
                )
                .with_arg("name", place.name.as_str().to_string()),
            );
        }
        if !place.ty.is_structurally_discardable()
            && !is_reference_type(self.typed, &place.ty)
            && !expression_consumes_place(self.typed, value, place_id)
        {
            self.typed.exprs.flaws[index].push(
                Diagnostic::new(
                    "type-rebind-would-discard-value",
                    format!(
                        "replacing `{}` requires consuming or transferring its previous value",
                        place.name.as_str()
                    ),
                )
                .with_arg("name", place.name.as_str().to_string()),
            );
        }
    }

    fn apply_condition_ownership_join(&mut self, paths: &[HashSet<ExprId>]) {
        let incoming_flow = self.flow_ctx.clone();
        let incoming_refs = self.ref_tracker.clone();
        let mut branch_flows = Vec::with_capacity(paths.len());
        let mut branch_refs = Vec::with_capacity(paths.len());
        for path in paths {
            *self.flow_ctx = incoming_flow.clone();
            *self.ref_tracker = incoming_refs.clone();
            for subject in path {
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
        ownership_paths: &[HashSet<ExprId>],
    ) {
        let incoming_flow = self.flow_ctx.clone();
        let incoming_refs = self.ref_tracker.clone();
        let incoming_bindings = self.const_bindings.clone();
        let incoming_values = self.local_const_values.clone();
        self.apply_condition_ownership_join(ownership_paths);
        for expr in branch {
            self.walk(*expr);
        }
        *self.flow_ctx = incoming_flow;
        *self.ref_tracker = incoming_refs;
        *self.const_bindings = incoming_bindings;
        *self.local_const_values = incoming_values;
    }

    fn walk_repeating_body(&mut self, body: &[ExprId], backedge: Option<Vec<ExprId>>) {
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
            for expression in backedge {
                self.walk(expression);
            }
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
        let idx = expr_id.index();
        let Some(kind) = self.typed.exprs.kind_of(expr_id).cloned() else {
            return;
        };

        // Save context for this expression
        let current_const_value = self.typed.exprs.const_value[idx].clone();
        self.typed.exprs.const_value[idx] = current_const_value;
        match kind {
            TypedExprKind::Lit(_) | TypedExprKind::Rematerialize { .. } => {}
            TypedExprKind::Binary { lhs, rhs, .. }
            | TypedExprKind::InvalidOperator { lhs, rhs, .. } => {
                self.walk(lhs);
                self.walk(rhs);
            }
            TypedExprKind::IntrinsicCall { args, .. } => {
                for arg in &args {
                    self.walk(*arg);
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
                if all_operands_anonymous_int && is_anonymous_result_int {
                    self.typed.exprs.const_value[idx] = None;
                }
            }
            TypedExprKind::FnCall {
                target,
                args,
                substituted_ty,
                ..
            } => {
                if args.as_ref().is_none_or(|a| a.is_empty())
                    && let Some(cv) = self.local_const_values.get(&target.0).cloned()
                {
                    self.typed.exprs.const_value[idx] = Some(cv);
                }
                if args.as_ref().is_none_or(|a| a.is_empty()) {
                    match self.typed.exprs.place[idx]
                        .and_then(|place| self.flow_ctx.place_state(place))
                    {
                        Some(VarState::Consumed) => {
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-of-moved-value",
                                    format!("use of moved value `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-after-move",
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
                            let mut diagnostic = Diagnostic::new(
                                "type-use-before-initialization",
                                format!("use of uninitialized local `{}`", target.0.as_str()),
                            )
                            .with_arg("name", target.0.as_str().to_string());
                            let declaration = self
                                .typed
                                .defs
                                .get(&target)
                                .or_else(|| {
                                    let place = self.typed.exprs.place[idx]?;
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
                                diagnostic.related.push(RelatedSpan {
                                    span: self.typed.span_table.get(bind.name_span),
                                    label: "declared without an initializer".to_string(),
                                });
                            } else if let Some(place) = self.typed.exprs.place[idx]
                                && let Some((expression, _)) = self
                                    .typed
                                    .exprs
                                    .place
                                    .iter()
                                    .enumerate()
                                    .find(|(_, candidate)| **candidate == Some(place))
                                && let Some(span) =
                                    self.typed.exprs.span_of(ExprId(expression as u32))
                            {
                                diagnostic.related.push(RelatedSpan {
                                    span: self.typed.span_table.get(span),
                                    label: "declared without an initializer".to_string(),
                                });
                            }
                            if let Some(place) = self.typed.exprs.place[idx] {
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
                                        diagnostic.related.push(RelatedSpan {
                                            span: self.typed.span_table.get(span),
                                            label: "initialized on this predecessor".to_string(),
                                        });
                                    }
                                }
                            }
                            self.typed.exprs.flaws[idx].push(diagnostic);
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
                        if matches!(
                            self.typed.exprs.kind_of(*arg_id),
                            Some(TypedExprKind::Ref(_) | TypedExprKind::ConsumeArg(_))
                        ) {
                            continue;
                        }
                        if !expr_is_rematerializable(self.typed, *arg_id) {
                            mark_consumed_if_local_ref(self.typed, *arg_id, self.flow_ctx);
                            if let Some(source) =
                                self.typed.exprs.target_group_of(*arg_id).cloned().flatten()
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
                            substituted_ty.as_ref(),
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
                        if !expr_is_rematerializable(self.typed, arg_id) {
                            mark_consumed_if_local_ref(self.typed, arg_id, self.flow_ctx);
                        }
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
                    if stmt.index() == idx {
                        continue;
                    }
                    self.walk(stmt);
                }
                if !unassigned && body.index() != idx {
                    self.walk(body);
                }
                // Track bind as a constant if its body has a const_value.
                let bind_const = self
                    .typed
                    .exprs
                    .const_value_of(body)
                    .cloned()
                    .flatten()
                    .or_else(|| self.typed.exprs.const_value[idx].clone());
                self.typed.exprs.const_value[idx] = bind_const.clone();
                if let Some(cv) = bind_const {
                    self.const_bindings.insert(name, true);
                    self.local_const_values.insert(name, cv);
                } else {
                    self.local_const_values.remove(&name);
                }
                let place = self.typed.exprs.place[idx];
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
                    }
                }
            }
            TypedExprKind::Reassign { name, value, .. } => {
                self.walk(value);
                let Some(target_group) = self
                    .typed
                    .exprs
                    .target_group_of(ExprId(idx as u32))
                    .and_then(|targets| targets.as_ref())
                    .cloned()
                else {
                    if self.typed.defs.contains_key(&crate::typed::DefId(name)) {
                        self.typed.exprs.flaws[idx].push(
                            Diagnostic::new(
                                "type-rebind-crosses-callable-boundary",
                                format!(
                                    "cannot rebind callable `{}` across scope boundary",
                                    name.as_str()
                                ),
                            )
                            .with_arg("name", name.as_str().to_string()),
                        );
                    } else {
                        self.typed.exprs.flaws[idx].push(
                            Diagnostic::new(
                                "type-rebind-target-not-found",
                                format!("unknown rebind target `{}`", name.as_str()),
                            )
                            .with_arg("name", name.as_str().to_string()),
                        );
                    }
                    return;
                };

                let target_place = self.typed.exprs.place[idx];
                let target_initialized = target_place
                    .and_then(|place| self.flow_ctx.place_state(place))
                    .is_some_and(|state| matches!(state, VarState::Declared));
                let target_ty = self.typed.exprs.place[idx]
                    .and_then(|place| self.typed.places.get(place.0 as usize))
                    .map(|place| &place.ty)
                    .or_else(|| self.local_var_types.get(&name));
                if !target_initialized
                    && let Some(ty) = target_ty
                    && !ty.is_structurally_discardable()
                    && !is_reference_type(self.typed, ty)
                    && !is_explicit_rebind_consumption(self.typed, value, name)
                {
                    self.typed.exprs.flaws[idx].push(
                        Diagnostic::new(
                            "type-rebind-would-discard-value",
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
                if let Some(place) = target_place {
                    self.flow_ctx.set_place_state(place, VarState::Alive);
                }

                let validity = target_ty.and_then(|ty| {
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
                let divides_upper_bound = target_ty.is_some_and(|ty| {
                    self.typed
                        .type_registry
                        .integer_validity_for_type(ty)
                        .or_else(|| ty.anonymous_integer_validity())
                        .is_some_and(|validity| refinement_divides_storage_max(validity.domain()))
                });
                if let Some((min, max)) = validity
                    && !rebind_rhs_within_integer_max(
                        &self.typed.exprs.kind,
                        &self.typed.exprs.const_value,
                        self.local_const_values,
                        name,
                        value,
                        min,
                        max,
                        divides_upper_bound,
                        &self.flow_ctx.constraint_env,
                    )
                {
                    self.typed.exprs.flaws[idx].push(
                        Diagnostic::new(
                            "type-rebind-violates-validity",
                            format!("rebind `{}` exceeds maximum bound", name.as_str()),
                        )
                        .with_arg("name", name.as_str().to_string()),
                    );
                }
            }
            TypedExprKind::Eat(inner) => {
                self.walk(inner);
                mark_consumed_if_local_ref(self.typed, inner, self.flow_ctx);
                if let Some(source) = self.typed.exprs.target_group_of(inner).cloned().flatten() {
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
                    self.typed
                        .exprs
                        .const_value_of(subject_id)
                        .and_then(Option::as_ref)
                        .cloned()
                });
                let mut selected_body = None;
                if let Some(subject_value) = subject_value.as_ref() {
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
                                    .is_some_and(|cv| *cv == *subject_value);
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
                    self.walk(body);
                    self.typed.exprs.const_value[idx] = self
                        .typed
                        .exprs
                        .const_value_of(body)
                        .cloned()
                        .unwrap_or(None);
                } else {
                    let incoming_flow = self.flow_ctx.clone();
                    let incoming_refs = self.ref_tracker.clone();
                    let incoming_bindings = self.const_bindings.clone();
                    let incoming_values = self.local_const_values.clone();
                    let mut branch_flows = Vec::with_capacity(when_expr.arms.len());
                    let mut branch_refs = Vec::with_capacity(when_expr.arms.len());
                    for arm in &when_expr.arms {
                        *self.flow_ctx = incoming_flow.clone();
                        *self.ref_tracker = incoming_refs.clone();
                        *self.const_bindings = incoming_bindings.clone();
                        *self.local_const_values = incoming_values.clone();
                        let body = match arm {
                            TypedWhenArm::Cond {
                                condition, body, ..
                            } => {
                                condition.visit_subjects(&mut |subject| self.walk(subject));
                                *body
                            }
                            TypedWhenArm::Is { pattern, body, .. } => {
                                if pattern_moves_subject(&pattern.value)
                                    && let Some(subject) = when_expr.subject
                                {
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
                        };
                        self.walk(body);
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
            }
            TypedExprKind::If(if_expr) => {
                let comparison_facts = condition_comparison_facts(
                    &if_expr.condition,
                    true,
                    &self.typed.exprs,
                    self.local_const_values,
                );
                let rejected_facts = condition_comparison_facts(
                    &if_expr.condition,
                    false,
                    &self.typed.exprs,
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
                let mut condition_subjects = Vec::new();
                if_expr
                    .condition
                    .visit_subjects(&mut |subject| condition_subjects.push(subject));
                for subject in condition_subjects {
                    self.walk(subject);
                }
                let taken_ownership = condition_ownership_paths(&if_expr.condition, true);
                let rejected_ownership = condition_ownership_paths(&if_expr.condition, false);
                let mut taken = if_expr.stmts;
                taken.extend(if_expr.ret);
                self.walk_terminating_condition_branch(&taken, &taken_ownership);
                // Restore constraint_env so facts don't leak past the if-block
                self.flow_ctx.constraint_env.known_lt.truncate(saved_lt_len);
                self.flow_ctx.constraint_env.known_le.truncate(saved_le_len);
                for (lhs, rhs, is_lt) in rejected_facts {
                    if is_lt {
                        self.flow_ctx.constraint_env.known_lt.push((lhs, rhs));
                    } else {
                        self.flow_ctx.constraint_env.known_le.push((lhs, rhs));
                    }
                }
                self.apply_condition_ownership_join(&rejected_ownership);
            }
            TypedExprKind::Loop(typed_loop) => {
                let saved_lt_len = self.flow_ctx.constraint_env.known_lt.len();
                let saved_le_len = self.flow_ctx.constraint_env.known_le.len();
                let backedge = match typed_loop.kind {
                    TypedLoopKind::While { ref condition, .. } => {
                        for (lhs, rhs, is_lt) in condition_comparison_facts(
                            condition,
                            true,
                            &self.typed.exprs,
                            self.local_const_values,
                        ) {
                            if is_lt {
                                self.flow_ctx.constraint_env.known_lt.push((lhs, rhs));
                            } else {
                                self.flow_ctx.constraint_env.known_le.push((lhs, rhs));
                            }
                        }
                        let mut subjects = Vec::new();
                        condition.visit_subjects(&mut |subject| subjects.push(subject));
                        for subject in &subjects {
                            self.walk(*subject);
                        }
                        Some(subjects)
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
                self.flow_ctx.constraint_env.known_lt.truncate(saved_lt_len);
                self.flow_ctx.constraint_env.known_le.truncate(saved_le_len);
            }
            TypedExprKind::TupleGet { base, .. } => {
                self.walk(base);
            }
            TypedExprKind::Destructure {
                tag_name,
                field_bindings,
                value,
            } => {
                self.walk(value);
                let value_ty = self
                    .typed
                    .exprs
                    .ty_of(value)
                    .and_then(|ty| reference_pointee_for_type(ty, Some(&self.typed.type_registry)))
                    .or_else(|| self.typed.exprs.ty_of(value).cloned())
                    .unwrap_or(Ty::Unit);
                if !is_reference_type(self.typed, &value_ty) {
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
                    self.typed.exprs.flaws[idx].push(
                        Diagnostic::new(
                            "type-pattern-would-discard-value",
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
                self.walk(base);
                self.walk(value);
                self.validate_projected_write(expr_id, value);
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
            TypedExprKind::TupleSet {
                base, index, value, ..
            } => {
                self.walk(base);
                self.walk(value);
                self.validate_projected_write(expr_id, value);
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
            TypedExprKind::TupleAlloc { init, .. } => {
                self.walk(init);
            }
            TypedExprKind::TupleLit(items) | TypedExprKind::List(items) => {
                for item_id in items {
                    self.walk(item_id);
                    if !expr_is_rematerializable(self.typed, item_id) {
                        mark_consumed_if_local_ref(self.typed, item_id, self.flow_ctx);
                    }
                }
            }
            TypedExprKind::BufGet { buf, index } => {
                self.walk(buf);
                self.walk(index);
            }
            TypedExprKind::BufSet {
                buf, index, value, ..
            } => {
                self.walk(buf);
                self.walk(index);
                self.walk(value);
                self.validate_projected_write(expr_id, value);
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
                if !matches!(
                    self.typed.exprs.kind[inner.as_usize()],
                    TypedExprKind::Rematerialize { .. }
                ) {
                    mark_consumed_if_local_ref(self.typed, inner, self.flow_ctx);
                    if let Some(source) = self.typed.exprs.target_group_of(inner).cloned().flatten()
                    {
                        self.ref_tracker
                            .permanently_invalidate(&source, self.flow_ctx);
                    }
                }
            }
            TypedExprKind::ConsumeArg(inner) => {
                self.walk(inner);
                mark_consumed_if_local_ref(self.typed, inner, self.flow_ctx);
                if let Some(source) = self.typed.exprs.target_group_of(inner).cloned().flatten() {
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
            TypedExprKind::SelfRef { .. } | TypedExprKind::TargetQuery { .. } => {}
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
                    let Some(span) = self.typed.exprs.span_of(*arg_id) else {
                        continue;
                    };
                    self.typed.exprs.flaws[arg_id.index()].push(
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
                    let Some(span) = self.typed.exprs.span_of(*arg_id) else {
                        continue;
                    };
                    self.typed.exprs.flaws[arg_id.index()].push(
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

fn expr_to_normal_expr(
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
            Literal::Number(n) => Some(NormalExpr::Value(ConstValue::Int((*n as u128).into()))),
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
        TypedExprKind::Cast { expr, .. } => expr_to_normal_expr(kinds, const_values, lcv, *expr),
        TypedExprKind::Rematerialize { source, .. } => {
            expr_to_normal_expr(kinds, const_values, lcv, *source)
        }
        TypedExprKind::TargetQuery { kind, operand } => Some(NormalExpr::TargetQuery {
            kind: *kind,
            operand: type_reference_for_ty(operand)?,
        }),
        TypedExprKind::FnCall { target, args, .. } if args.is_none() => lcv
            .get(&target.0)
            .cloned()
            .map(NormalExpr::Value)
            .or(Some(NormalExpr::Var(target.0))),
        TypedExprKind::FnCall { target, args, .. }
            if args.as_ref().is_some_and(|a| a.len() == 2) =>
        {
            let args = args.as_deref().expect("checked args len above");
            let TargetIndex::Symbolic(left) =
                expr_to_target_index(kinds, const_values, lcv, args[0])
            else {
                return None;
            };
            let TargetIndex::Symbolic(right) =
                expr_to_target_index(kinds, const_values, lcv, args[1])
            else {
                return None;
            };
            match target.0.as_str() {
                "add" => Some(NormalExpr::Add(Box::new(left), Box::new(right))),
                "sub" => Some(NormalExpr::Sub(Box::new(left), Box::new(right))),
                "mul" => Some(NormalExpr::Mul(Box::new(left), Box::new(right))),
                _ => None,
            }
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

fn expr_to_target_index(
    kinds: &[TypedExprKind],
    const_values: &[Option<ConstValue>],
    _lcv: &HashMap<Intern<String>, ConstValue>,
    expr_id: ExprId,
) -> TargetIndex {
    let idx = expr_id.index();
    if let Some(value) = const_values.get(idx).and_then(Clone::clone) {
        return TargetIndex::Symbolic(ast::NormalExpr::Value(value));
    }
    let Some(kind) = kinds.get(idx) else {
        return TargetIndex::Unknown;
    };
    let expr = match kind {
        TypedExprKind::Lit(Literal::Int(value)) => {
            ast::NormalExpr::Value(ast::ConstValue::Int(*value))
        }
        TypedExprKind::Lit(Literal::Number(value)) => {
            ast::NormalExpr::Value(ast::ConstValue::Int((*value as u128).into()))
        }
        TypedExprKind::Ref(inner)
        | TypedExprKind::TakePtr(inner)
        | TypedExprKind::ConsumeArg(inner)
        | TypedExprKind::Eat(inner)
        | TypedExprKind::Deref(inner) => {
            return expr_to_target_index(kinds, const_values, _lcv, *inner);
        }
        TypedExprKind::Cast { expr, .. } => {
            return expr_to_target_index(kinds, const_values, _lcv, *expr);
        }
        TypedExprKind::Rematerialize { source, .. } => {
            return expr_to_target_index(kinds, const_values, _lcv, *source);
        }
        TypedExprKind::TargetQuery { kind, operand } => ast::NormalExpr::TargetQuery {
            kind: *kind,
            operand: match type_reference_for_ty(operand) {
                Some(operand) => operand,
                None => return TargetIndex::Unknown,
            },
        },
        TypedExprKind::FnCall { target, args, .. } if args.is_none() => {
            ast::NormalExpr::Var(target.0)
        }
        TypedExprKind::FnCall { target, args, .. }
            if args.as_ref().is_some_and(|a| a.len() == 2) =>
        {
            let args = args.as_deref().expect("checked args len above");
            let TargetIndex::Symbolic(left) =
                expr_to_target_index(kinds, const_values, _lcv, args[0])
            else {
                return TargetIndex::Unknown;
            };
            let TargetIndex::Symbolic(right) =
                expr_to_target_index(kinds, const_values, _lcv, args[1])
            else {
                return TargetIndex::Unknown;
            };
            match target.0.as_str() {
                "add" => ast::NormalExpr::Add(Box::new(left), Box::new(right)),
                "sub" => ast::NormalExpr::Sub(Box::new(left), Box::new(right)),
                "mul" => ast::NormalExpr::Mul(Box::new(left), Box::new(right)),
                _ => return TargetIndex::Unknown,
            }
        }
        TypedExprKind::IntrinsicCall { op, args } if args.len() == 2 => {
            let TargetIndex::Symbolic(left) =
                expr_to_target_index(kinds, const_values, _lcv, args[0])
            else {
                return TargetIndex::Unknown;
            };
            let TargetIndex::Symbolic(right) =
                expr_to_target_index(kinds, const_values, _lcv, args[1])
            else {
                return TargetIndex::Unknown;
            };
            match op {
                crate::typed::IntrinsicOp::BitsAdd => {
                    ast::NormalExpr::Add(Box::new(left), Box::new(right))
                }
                crate::typed::IntrinsicOp::BitsSubtract => {
                    ast::NormalExpr::Sub(Box::new(left), Box::new(right))
                }
                crate::typed::IntrinsicOp::BitsMultiply => {
                    ast::NormalExpr::Mul(Box::new(left), Box::new(right))
                }
                _ => return TargetIndex::Unknown,
            }
        }
        _ => return TargetIndex::Unknown,
    };
    TargetIndex::Symbolic(expr)
}

fn type_reference_for_ty(ty: &Ty) -> Option<ast::TypeReference> {
    match ty {
        Ty::Unit => Some(ast::TypeReference::Unit),
        Ty::Opaque(name) => Some(ast::TypeReference::Nominal(*name)),
        Ty::Ptr { inner } => Some(ast::TypeReference::Pointer(Box::new(
            type_reference_for_ty(inner)?,
        ))),
        Ty::Named { name, instance, .. } => Some(ast::TypeReference::Application {
            name: *name,
            arguments: instance
                .arguments
                .iter()
                .filter_map(|(_, argument)| match argument {
                    ast::TyArg::Type(ty) => type_reference_for_ty(ty),
                    ast::TyArg::Const(_) => None,
                })
                .collect(),
        }),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn rebind_rhs_within_integer_max(
    kinds: &[TypedExprKind],
    const_values: &[Option<ConstValue>],
    lcv: &HashMap<Intern<String>, ConstValue>,
    local_name: Intern<String>,
    rhs: ExprId,
    min: i128,
    max: i128,
    divides_upper_bound: bool,
    constraints: &ConstraintEnv,
) -> bool {
    let Some(rhs_expr) = expr_to_normal_expr(kinds, const_values, lcv, rhs) else {
        return false;
    };
    if let NormalExpr::Value(ConstValue::Int(value)) = &rhs_expr {
        return I256::from(min) <= *value && *value <= I256::from(max);
    }
    let known_vars = lcv
        .iter()
        .filter_map(|(name, value)| value.as_const_size_int().map(|value| (*name, value)))
        .map(|(name, value)| (name, NormalExpr::Value(ConstValue::Int(value.into()))))
        .collect::<HashMap<_, _>>();
    if matches!(
        constraints.prove(
            &Predicate::Lt(
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
        NormalExpr::Add(lhs, rhs) if *lhs == NormalExpr::Var(local_name) => {
            let Some(delta) = rhs.as_const_int() else {
                return false;
            };
            constraints.prove(
                &Predicate::Lt(
                    *lhs,
                    NormalExpr::Value(ConstValue::Int((max - delta).into())),
                ),
                &known_vars,
            ) == ProveResult::Proven
        }
        NormalExpr::Mul(lhs, rhs)
            if divides_upper_bound
                && *lhs == NormalExpr::Var(local_name)
                && rhs.as_const_int() == Some(2) =>
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

fn condition_comparison_facts(
    condition: &crate::typed::TypedCondition,
    taken: bool,
    expressions: &crate::typed::TypedExprVec,
    local_values: &HashMap<Intern<String>, ConstValue>,
) -> Vec<(NormalExpr, NormalExpr, bool)> {
    match condition {
        crate::typed::TypedCondition::Is { subject, pattern } => {
            let ast::Pattern::Nominal(label, _) = &pattern.value else {
                return Vec::new();
            };
            let selected = if taken {
                *label
            } else if label.as_str() == "True" {
                Intern::from_ref("False")
            } else if label.as_str() == "False" {
                Intern::from_ref("True")
            } else {
                return Vec::new();
            };
            let Some(evidence) = expressions
                .result_evidence_of(*subject)
                .and_then(Option::as_ref)
            else {
                return Vec::new();
            };
            let Some(alternative) = evidence
                .alternatives
                .iter()
                .find(|alternative| alternative.label == selected)
            else {
                return Vec::new();
            };
            let crate::typed::EvidenceProposition::IntegerComparison {
                lhs,
                rhs,
                comparison,
            } = alternative.proposition;
            let Some(lhs) = expr_to_normal_expr(
                &expressions.kind,
                &expressions.const_value,
                local_values,
                lhs,
            ) else {
                return Vec::new();
            };
            let Some(rhs) = expr_to_normal_expr(
                &expressions.kind,
                &expressions.const_value,
                local_values,
                rhs,
            ) else {
                return Vec::new();
            };
            use crate::typed::MathematicalComparison as Math;
            match comparison {
                Math::Less => vec![(lhs, rhs, true)],
                Math::LessOrEqual => vec![(lhs, rhs, false)],
                Math::Greater => vec![(rhs, lhs, true)],
                Math::GreaterOrEqual => vec![(rhs, lhs, false)],
                Math::Equal | Math::NotEqual => Vec::new(),
            }
        }
        crate::typed::TypedCondition::Not(inner) => {
            condition_comparison_facts(inner, !taken, expressions, local_values)
        }
        crate::typed::TypedCondition::And(left, right) if taken => {
            let mut facts = condition_comparison_facts(left, true, expressions, local_values);
            facts.extend(condition_comparison_facts(
                right,
                true,
                expressions,
                local_values,
            ));
            facts
        }
        crate::typed::TypedCondition::Or(left, right) if !taken => {
            let mut facts = condition_comparison_facts(left, false, expressions, local_values);
            facts.extend(condition_comparison_facts(
                right,
                false,
                expressions,
                local_values,
            ));
            facts
        }
        crate::typed::TypedCondition::And(_, _) | crate::typed::TypedCondition::Or(_, _) => {
            Vec::new()
        }
    }
}

fn condition_ownership_paths(
    condition: &crate::typed::TypedCondition,
    taken: bool,
) -> Vec<HashSet<ExprId>> {
    match condition {
        crate::typed::TypedCondition::Is { subject, pattern } => {
            let mut path = HashSet::new();
            if taken && pattern_moves_subject(&pattern.value) {
                path.insert(*subject);
            }
            vec![path]
        }
        crate::typed::TypedCondition::Not(inner) => condition_ownership_paths(inner, !taken),
        crate::typed::TypedCondition::And(left, right) if taken => combine_ownership_paths(
            condition_ownership_paths(left, true),
            condition_ownership_paths(right, true),
        ),
        crate::typed::TypedCondition::And(left, right) => {
            let mut paths = condition_ownership_paths(left, false);
            paths.extend(combine_ownership_paths(
                condition_ownership_paths(left, true),
                condition_ownership_paths(right, false),
            ));
            paths
        }
        crate::typed::TypedCondition::Or(left, right) if taken => {
            let mut paths = condition_ownership_paths(left, true);
            paths.extend(combine_ownership_paths(
                condition_ownership_paths(left, false),
                condition_ownership_paths(right, true),
            ));
            paths
        }
        crate::typed::TypedCondition::Or(left, right) => combine_ownership_paths(
            condition_ownership_paths(left, false),
            condition_ownership_paths(right, false),
        ),
    }
}

fn combine_ownership_paths(
    left: Vec<HashSet<ExprId>>,
    right: Vec<HashSet<ExprId>>,
) -> Vec<HashSet<ExprId>> {
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

fn pattern_moves_subject(pattern: &ast::Pattern) -> bool {
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
        ast::Pattern::Tuple(items) => items.iter().any(|item| pattern_moves_subject(&item.value)),
        ast::Pattern::ListCons { head, tail } => {
            pattern_moves_subject(&head.value) || pattern_moves_subject(&tail.value)
        }
        ast::Pattern::Ref { .. }
        | ast::Pattern::Qualified(_)
        | ast::Pattern::Literal(_, _)
        | ast::Pattern::Unit
        | ast::Pattern::ListEmpty
        | ast::Pattern::InRange { .. } => false,
    }
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
                    Box::new(Ty::Tuple(vec![
                        Ty::anonymous_integer_for_width(8, false),
                        Ty::Unit,
                    ])),
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
        let parsed =
            TokenCursor::parse_source("main:\n    if 1 < 2 is True\n        1\n    return 0\n");
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        let body = match &typed.defs[&DefId(Intern::from_ref("main"))].body {
            BindBody::Body { exprs, .. } => exprs[0],
            _ => panic!("expected body"),
        };
        let TypedExprKind::If(if_expression) = &typed.exprs.kind[body.as_usize()] else {
            panic!("expected if expression");
        };

        let taken = condition_comparison_facts(
            &if_expression.condition,
            true,
            &typed.exprs,
            &HashMap::new(),
        );
        let rejected = condition_comparison_facts(
            &if_expression.condition,
            false,
            &typed.exprs,
            &HashMap::new(),
        );

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
            "Int is in 0...255\nChoice is Some(Int) or None\nkeep(value Choice) Choice:\n    if value is Some(payload)\n        Choice.None\n    return\n    return value",
        );
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        let codes = typed
            .all_flaws()
            .into_iter()
            .map(|(_, flaw)| flaw.code.slug().to_string())
            .collect::<Vec<_>>();

        assert!(
            !codes.iter().any(|code| code.contains("moved-value")),
            "{codes:?}"
        );
    }

    #[test]
    fn successful_by_value_condition_binding_moves_subject_inside_branch() {
        let parsed = TokenCursor::parse_source(
            "Int is in 0...255\nChoice is Some(Int) or None\nbad(value Choice) Choice:\n    if value is Some(payload)\n        value\n    return\n    return value",
        );
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        let codes = typed
            .all_flaws()
            .into_iter()
            .map(|(_, flaw)| flaw.code.slug().to_string())
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
#[path = "../../tests/reference_validity_tests.rs"]
mod reference_validity_tests;
