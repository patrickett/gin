//! Stage 2: Resolve — Lower parse expressions to the typed arena, attach type flaws.
//!
//! This stage walks the parse-tree expressions, infers/resolves their types,
//! converts them to [`TypedExprKind`], pushes them into the expression arena,
//! and attaches type-check flaws.

use crate::tag_declarations::TagDeclarations;
use diagnostic::code::{
    AddressCode, ArrayCode, BindingCode, CallCode, GenericCode, MemberCode, OperatorCode,
    OwnershipCode, PatternCode, PropositionCode, ResultCode, SequenceCode, SymbolCode, TextCode,
    TypeCode, UnionCode,
};
use internment::Intern;
use std::collections::{HashMap, HashSet};

use crate::analysis::when_is_exhaustive;
use ast::prelude::*;
use ast::type_decl::TypeNameExt;
use ast_format::type_expr::ExprFormatExt;

use crate::Normalize;
use crate::analysis::TypeEnv;
use crate::intrinsic::IntrinsicOp;
use crate::operator::OperatorRegistry;
use crate::ty::{IntegerInterpretation, Ty, reference_pointee_for_type};
use crate::typed::{
    Availability, BindBody, DefId, ExprId, ReferenceTargetGroup, ReferenceTargetSet, TagId,
    TargetIndex, TypedCallableSignature, TypedExprKind, TypedFileAst, TypedWhenArm, VariantMap,
};
use ast::ConstValue;
use diagnostic::{Diagnostic, DiagnosticCode};

use super::bounds_check::check_fn_call_bounds;
use super::lower_kind::lower_expr_kind;
use super::lower_tag::is_record_tag_call;
use super::lower_ty::{
    ExprTypeResolverCtx, annotate_literal_union_literal, bind_explicit_ty_in_scope,
    resolve_expr_type,
};
use super::lower_util::closest_name;
use super::{TransformCtx, flaw};

fn integer_width_for_exact_value(value: i256::I256) -> u8 {
    ast::integer::FiniteHull::new(value, value)
        .map(|hull| hull.inferred_representation().width().get())
        .and_then(|width| u8::try_from(width).ok())
        .unwrap_or(u8::MAX)
}

fn named_instance_has_symbolic_argument(instance: &ast::ty::NamedTypeInstance) -> bool {
    instance.arguments.iter().any(|(_, argument)| {
        matches!(argument, ast::ty::TyArg::Type(ty) if matches!(ty.as_ref(), Ty::Opaque(_)))
    })
}

/// Map from local variable name to its resolved type, built incrementally
/// during expression lowering. This lets `resolve_expr_type` look up the
/// type of a variable that was bound earlier in the same function body.
pub(crate) type LocalVarTypes = HashMap<Intern<String>, Ty>;

#[derive(Clone)]
pub(crate) struct LocalCallable {
    pub(crate) definition: ExprId,
    pub(crate) return_type: Ty,
    pub(crate) parameters: Vec<(Intern<String>, Ty)>,
    pub(crate) defaults: Vec<Option<Typed<Expr>>>,
}

/// Local types plus names introduced with `:=` (immutable).
#[derive(Clone, Default)]
pub(crate) struct FlowState {
    pub(crate) types: LocalVarTypes,
    /// Names declared in the current scope (for reassignment detection).
    pub(crate) locals: HashSet<Intern<String>>,
    pub(crate) places: HashMap<Intern<String>, crate::typed::PlaceId>,
    pub(crate) place_versions: HashMap<Intern<String>, crate::typed::PlaceVersionId>,
    pub(crate) projected_places:
        HashMap<(crate::typed::PlaceId, crate::typed::PlaceProjection), crate::typed::PlaceId>,
    pub(crate) projected_versions: HashMap<crate::typed::PlaceId, crate::typed::PlaceVersionId>,
    /// Names introduced with `:=` (immutable) for the `ReassignConstant` check.
    pub(crate) constants: HashSet<Intern<String>>,
    /// Compile-time-known local values available while lowering this scope.
    pub(crate) const_values: HashMap<Intern<String>, ast::ConstValue>,
    pub(crate) target_groups: HashMap<Intern<String>, ReferenceTargetSet>,
    pub(crate) local_callables: HashMap<Intern<String>, LocalCallable>,
    pub(crate) result_evidence: HashMap<Intern<String>, crate::typed::ValueEvidence>,
    pub(crate) union_members: HashMap<crate::typed::PlaceVersionId, (usize, Ty)>,
    pub(crate) integer_domains: HashMap<crate::typed::PlaceVersionId, ast::integer::IntegerDomain>,
    pub(crate) projected_result_evidence:
        HashMap<Intern<String>, crate::typed::ProjectedResultEvidence>,
    /// Result labels carried by local bindings initialized from callable rows.
    pub(crate) result_names: HashMap<Intern<String>, Vec<Option<Intern<String>>>>,
    /// Per-field ownership targets carried by stored callable result rows.
    pub(crate) result_target_groups: HashMap<Intern<String>, Vec<Option<ReferenceTargetSet>>>,
}

impl FlowState {
    pub(crate) fn remove_binding(&mut self, name: Intern<String>) {
        self.types.remove(&name);
        self.locals.remove(&name);
        self.places.remove(&name);
        self.place_versions.remove(&name);
        self.constants.remove(&name);
        self.const_values.remove(&name);
        self.target_groups.remove(&name);
        self.local_callables.remove(&name);
        self.result_evidence.remove(&name);
        self.projected_result_evidence.remove(&name);
        self.result_names.remove(&name);
        self.result_target_groups.remove(&name);
    }

    pub(crate) fn invalidate_mutable_constants(&mut self) {
        self.const_values
            .retain(|name, _| self.constants.contains(name));
    }

    fn invalidate_value_evidence(&mut self, typed: &mut TypedFileAst, call: ExprId) {
        self.integer_domains.clear();
        self.union_members.clear();
        self.invalidate_mutable_constants();
        let mut places: Vec<_> = self
            .places
            .iter()
            .map(|(name, place)| (*name, *place))
            .collect();
        places.sort_by_key(|(_, place)| place.0);
        for (name, place) in places {
            let ty = typed.places[place.0 as usize].ty.clone();
            let version = typed.add_place_version(
                place,
                crate::typed::PlaceVersionOrigin::Mutation(call),
                ty,
                None,
            );
            self.set_place_version(name, version);
        }
        self.projected_versions.clear();
    }

    pub(crate) fn place_version(
        &self,
        name: &Intern<String>,
    ) -> Option<crate::typed::PlaceVersionId> {
        self.place_versions.get(name).copied()
    }

    pub(crate) fn set_place_version(
        &mut self,
        name: Intern<String>,
        version: crate::typed::PlaceVersionId,
    ) {
        self.place_versions.insert(name, version);
    }

    pub(crate) fn set_target_group(
        &mut self,
        name: Intern<String>,
        targets: Option<ReferenceTargetSet>,
    ) {
        if let Some(targets) = targets {
            self.target_groups.insert(name, targets);
        } else {
            self.target_groups.remove(&name);
        }
    }

    pub(crate) fn set_result_target_groups(
        &mut self,
        name: Intern<String>,
        targets: Option<Vec<Option<ReferenceTargetSet>>>,
    ) {
        if let Some(targets) = targets.filter(|targets| !targets.is_empty()) {
            self.result_target_groups.insert(name, targets);
        } else {
            self.result_target_groups.remove(&name);
        }
    }

    pub(crate) fn projected_place(
        &self,
        parent: crate::typed::PlaceId,
        projection: &crate::typed::PlaceProjection,
    ) -> Option<crate::typed::PlaceId> {
        self.projected_places
            .get(&(parent, projection.clone()))
            .copied()
    }

    pub(crate) fn set_projected_place(
        &mut self,
        parent: crate::typed::PlaceId,
        projection: crate::typed::PlaceProjection,
        place: crate::typed::PlaceId,
    ) {
        self.projected_places.insert((parent, projection), place);
    }

    pub(crate) fn projected_version(
        &self,
        place: crate::typed::PlaceId,
    ) -> Option<crate::typed::PlaceVersionId> {
        self.projected_versions.get(&place).copied()
    }

    pub(crate) fn set_projected_version(
        &mut self,
        place: crate::typed::PlaceId,
        version: crate::typed::PlaceVersionId,
    ) {
        self.projected_versions.insert(place, version);
    }
}

pub(crate) fn join_place_versions(
    typed: &mut TypedFileAst,
    env: &mut FlowState,
    incoming: &FlowState,
    branches: &[FlowState],
    include_incoming: bool,
) {
    env.union_members.retain(|version, member| {
        branches
            .iter()
            .all(|branch| branch.union_members.get(version) == Some(member))
    });
    env.integer_domains.retain(|version, domain| {
        branches
            .iter()
            .all(|branch| branch.integer_domains.get(version) == Some(domain))
    });
    let mut places: Vec<_> = incoming.places.iter().collect();
    places.sort_by_key(|(_, place)| place.0);
    for (name, place) in places {
        let Some(incoming_version) = incoming.place_version(name) else {
            continue;
        };
        let mut predecessors = Vec::new();
        if include_incoming {
            predecessors.push(incoming_version);
        }
        for branch in branches {
            let version = if branch.places.get(name) == Some(place) {
                branch
                    .place_versions
                    .get(name)
                    .copied()
                    .unwrap_or(incoming_version)
            } else {
                incoming_version
            };
            if !predecessors.contains(&version) {
                predecessors.push(version);
            }
        }
        if predecessors.len() == 1 {
            env.set_place_version(*name, predecessors[0]);
            continue;
        }
        let version = typed.add_place_version(
            *place,
            crate::typed::PlaceVersionOrigin::Join(predecessors),
            typed.places[place.0 as usize].ty.clone(),
            None,
        );
        env.set_place_version(*name, version);
    }
    let projected_keys = incoming
        .projected_places
        .keys()
        .chain(
            branches
                .iter()
                .flat_map(|branch| branch.projected_places.keys()),
        )
        .cloned()
        .collect::<HashSet<_>>();
    let mut projected_places: Vec<_> = projected_keys
        .into_iter()
        .filter_map(|(parent, projection)| {
            let place = incoming
                .projected_places
                .get(&(parent, projection.clone()))
                .or_else(|| {
                    branches.iter().find_map(|branch| {
                        branch.projected_places.get(&(parent, projection.clone()))
                    })
                })
                .copied()?;
            Some((parent, projection, place))
        })
        .collect();
    projected_places.sort_by_key(|(_, _, place)| place.0);
    for (parent, projection, place) in projected_places {
        let incoming_version = if let Some(version) = incoming.projected_version(place) {
            version
        } else {
            let Some(base) = current_version_for_place(typed, incoming, parent) else {
                continue;
            };
            typed.add_place_version(
                place,
                crate::typed::PlaceVersionOrigin::Projection { base },
                typed.places[place.0 as usize].ty.clone(),
                None,
            )
        };
        let mut predecessors = Vec::new();
        if include_incoming {
            predecessors.push(incoming_version);
        }
        for branch in branches {
            let version = branch
                .projected_versions
                .get(&place)
                .copied()
                .unwrap_or(incoming_version);
            if !predecessors.contains(&version) {
                predecessors.push(version);
            }
        }
        let version = if predecessors.len() == 1 {
            predecessors[0]
        } else {
            typed.add_place_version(
                place,
                crate::typed::PlaceVersionOrigin::Join(predecessors),
                typed.places[place.0 as usize].ty.clone(),
                None,
            )
        };
        env.set_projected_place(parent, projection, place);
        env.set_projected_version(place, version);
    }
}

pub(crate) fn prepare_loop_place_versions(typed: &mut TypedFileAst, env: &mut FlowState) {
    let mut inputs: Vec<_> = env
        .places
        .iter()
        .filter_map(|(name, place)| {
            let ty = &typed.places[place.0 as usize].ty;
            let incoming = env.place_version(name)?;
            (!env.constants.contains(name)
                && typed.type_registry.integer_validity_for_type(ty).is_some())
            .then(|| (*name, *place, incoming, ty.clone()))
        })
        .collect();
    inputs.sort_by_key(|(_, place, _, _)| place.0);
    for (name, place, incoming, ty) in inputs {
        let version = typed.add_place_version(
            place,
            crate::typed::PlaceVersionOrigin::Join(vec![incoming]),
            ty,
            None,
        );
        env.set_place_version(name, version);
    }
}

pub(crate) fn join_loop_place_versions(
    typed: &mut TypedFileAst,
    env: &mut FlowState,
    incoming: &FlowState,
    body: &FlowState,
) {
    let mut places: Vec<_> = incoming.places.iter().collect();
    places.sort_by_key(|(_, place)| place.0);
    for (name, place) in places {
        let Some(incoming_version) = incoming.place_version(name) else {
            continue;
        };
        let backedge = if body.places.get(name) == Some(place) {
            body.place_versions.get(name).copied()
        } else {
            None
        };
        let Some(backedge) = backedge.filter(|version| *version != incoming_version) else {
            env.set_place_version(*name, incoming_version);
            continue;
        };
        let version = typed.add_place_version(
            *place,
            crate::typed::PlaceVersionOrigin::LoopPhi {
                incoming: incoming_version,
                backedges: vec![backedge],
            },
            typed.places[place.0 as usize].ty.clone(),
            None,
        );
        close_loop_version_cycle(typed, backedge, incoming_version, version, *place);
        env.set_place_version(*name, version);
    }
    let mut projected_places: Vec<_> = body.projected_places.iter().collect();
    projected_places.sort_by_key(|(_, place)| place.0);
    for ((parent, projection), place) in projected_places {
        let incoming_version = if let Some(version) = incoming.projected_version(*place) {
            version
        } else {
            let Some(base) = current_version_for_place(typed, incoming, *parent) else {
                continue;
            };
            typed.add_place_version(
                *place,
                crate::typed::PlaceVersionOrigin::Projection { base },
                typed.places[place.0 as usize].ty.clone(),
                None,
            )
        };
        let Some(backedge) = body
            .projected_versions
            .get(place)
            .copied()
            .filter(|version| *version != incoming_version)
        else {
            env.projected_places
                .insert((*parent, projection.clone()), *place);
            env.set_projected_version(*place, incoming_version);
            continue;
        };
        let version = typed.add_place_version(
            *place,
            crate::typed::PlaceVersionOrigin::LoopPhi {
                incoming: incoming_version,
                backedges: vec![backedge],
            },
            typed.places[place.0 as usize].ty.clone(),
            None,
        );
        close_loop_version_cycle(typed, backedge, incoming_version, version, *place);
        env.projected_places
            .insert((*parent, projection.clone()), *place);
        env.set_projected_version(*place, version);
    }
}

fn close_loop_version_cycle(
    typed: &mut TypedFileAst,
    backedge: crate::typed::PlaceVersionId,
    incoming: crate::typed::PlaceVersionId,
    phi: crate::typed::PlaceVersionId,
    place: crate::typed::PlaceId,
) {
    for version in [backedge, incoming, phi] {
        assert_eq!(
            typed.place_versions[version.0 as usize].place, place,
            "loop cycle versions must belong to the loop-carried place"
        );
    }
    let mut pending = vec![backedge];
    let mut visited = HashSet::new();
    while let Some(version) = pending.pop() {
        if !visited.insert(version) || typed.place_versions[version.0 as usize].place != place {
            continue;
        }
        let origin = &mut typed.place_versions[version.0 as usize].origin;
        match origin {
            crate::typed::PlaceVersionOrigin::Rebind { predecessor, .. } => {
                if *predecessor == Some(incoming) {
                    *predecessor = Some(phi);
                } else if let Some(predecessor) = *predecessor {
                    pending.push(predecessor);
                }
            }
            crate::typed::PlaceVersionOrigin::Join(predecessors) => {
                for predecessor in predecessors {
                    if *predecessor == incoming {
                        *predecessor = phi;
                    } else {
                        pending.push(*predecessor);
                    }
                }
            }
            crate::typed::PlaceVersionOrigin::LoopPhi {
                incoming,
                backedges,
            } => {
                pending.push(*incoming);
                pending.extend(backedges.iter().copied());
            }
            crate::typed::PlaceVersionOrigin::Parameter
            | crate::typed::PlaceVersionOrigin::Declared
            | crate::typed::PlaceVersionOrigin::Initializer(_)
            | crate::typed::PlaceVersionOrigin::Temporary(_)
            | crate::typed::PlaceVersionOrigin::Mutation(_) => {}
            crate::typed::PlaceVersionOrigin::Projection { base } => pending.push(*base),
        }
    }
}

/// Lexical context while lowering expressions.
pub(crate) struct ExprLowerScope<'a> {
    pub(crate) tag_types: &'a HashMap<Intern<String>, Ty>,
    pub(crate) variant_map: &'a VariantMap,
    pub(crate) receiver_type: Option<&'a Ty>,
    /// When set (e.g. explicit function return type), bare tags like `True` resolve as that union's variants.
    pub(crate) expected_ty: Option<&'a Ty>,
    pub(crate) return_ty: Option<&'a Ty>,
    pub(crate) value_required: bool,
    pub(crate) when_subject: Option<(SpanId, ExprId)>,
    pub(crate) locals: &'a HashSet<Intern<String>>,
    /// Current definition being lowered (for SelfRef resolution).
    pub(crate) current_def_id: Option<DefId>,
    pub(crate) overload_groups: &'a ast::OverloadGroups,
    pub(crate) callable_signatures: &'a HashMap<DefId, TypedCallableSignature>,
    pub(crate) operator_registry: &'a OperatorRegistry<'a>,
    pub(crate) tag_params: &'a HashMap<Intern<String>, Parameters>,
    pub(crate) tag_decls: TagDeclarations<'a>,
    pub(crate) source_defs: &'a ast::DefMap,
    pub(crate) implementation_reads:
        Option<&'a super::implementation_reads::ImplementationReadRecorder>,
    pub(crate) constant_queries: &'a super::constant_queries::ConstantQueries,
    pub(crate) allow_literal_materialization: bool,
}

impl ExprLowerScope<'_> {
    /// Call args, binary operands, and nested `when` subjects do not inherit the outer expected return type.
    pub(crate) fn child(&self) -> Self {
        Self {
            tag_types: self.tag_types,
            variant_map: self.variant_map,
            receiver_type: self.receiver_type,
            expected_ty: None,
            return_ty: self.return_ty,
            value_required: true,
            when_subject: self.when_subject,
            locals: self.locals,
            current_def_id: self.current_def_id,
            callable_signatures: self.callable_signatures,
            overload_groups: self.overload_groups,
            operator_registry: self.operator_registry,
            tag_params: self.tag_params,
            tag_decls: self.tag_decls,
            source_defs: self.source_defs,
            implementation_reads: self.implementation_reads,
            constant_queries: self.constant_queries,
            allow_literal_materialization: true,
        }
    }

    pub(crate) fn with_expected<'a>(&'a self, expected_ty: Option<&'a Ty>) -> ExprLowerScope<'a> {
        ExprLowerScope {
            expected_ty,
            value_required: self.value_required,
            ..self.child()
        }
    }

    pub(crate) fn statement(&self) -> Self {
        let mut scope = self.child();
        scope.value_required = false;
        scope
    }

    pub(crate) fn without_literal_materialization(&self) -> Self {
        Self {
            allow_literal_materialization: false,
            ..self.child()
        }
    }

    pub(crate) fn with_anonymous_literal_materialization(&self) -> Self {
        Self {
            allow_literal_materialization: true,
            ..self.child()
        }
    }
}

pub(super) fn stage_lower_with_queries(
    typed: &mut TypedFileAst,
    file_ast: &FileAst,
    ctx: &TransformCtx,
    constant_queries: &super::constant_queries::ConstantQueries,
    implementation_reads: Option<&super::implementation_reads::ImplementationReadRecorder>,
) {
    let defaults = super::trait_methods::TraitMethods::new(file_ast, &ctx.compile_time_eval_ast)
        .with_reads(implementation_reads)
        .expand_defaults();
    let file_ast = defaults.as_ref();
    // Own tag types plus cross-file.
    let mut tag_types: HashMap<Intern<String>, Ty> = ctx
        .cross_file_tag_types
        .iter()
        .map(|(tid, ty)| (tid.0, ty.clone()))
        .collect();
    for (tid, ty) in typed.tag_types.iter() {
        tag_types.insert(tid.0, ty.clone());
    }
    // Collect variant_map reference before any mutable borrows (own + cross-file).
    let mut variant_map: VariantMap = (*typed.variant_map).clone();
    super::extend_variant_map_from_ctx(&mut variant_map, ctx);

    // First pass: lower all expressions, collecting body assignments.
    struct DefBodyAssign {
        def_id: DefId,
        body: BindBody,
    }

    let mut def_ids: Vec<DefId> = typed.defs.keys().copied().collect();
    def_ids.sort_by_key(|def_id| {
        let bind = file_ast.defs.get(def_id);
        let is_value_expr = bind
            .is_some_and(|bind| matches!(bind.value, BindValue::Expr(_)) && bind.params.is_none());
        let source_start = bind
            .map(|bind| typed.span_table.get(bind.name_span).start())
            .unwrap_or(usize::MAX);
        (!is_value_expr, source_start, def_id.0.as_str().to_string())
    });
    super::return_types::order_return_dependencies(&mut def_ids, file_ast);

    // Pre-collect receiver types and param names.
    let receiver_types: HashMap<DefId, Option<Ty>> = def_ids
        .iter()
        .map(|def_id| {
            let recv = typed.defs.get(def_id).and_then(|b| b.receiver_type.clone());
            (*def_id, recv)
        })
        .collect();

    let param_sets: HashMap<DefId, HashSet<Intern<String>>> = def_ids
        .iter()
        .map(|def_id| {
            let mut params: HashSet<Intern<String>> = typed
                .defs
                .get(def_id)
                .map(|b| b.params.iter().map(|(n, _)| *n).collect())
                .unwrap_or_default();
            if let Some(parsed_params) = file_ast
                .defs
                .get(def_id)
                .and_then(|bind| bind.params.as_ref())
            {
                params.extend(parsed_params.keys().copied());
            }
            (*def_id, params)
        })
        .collect();

    let mut tag_params = ctx.cross_file_tag_params.clone();
    tag_params.extend(
        typed
            .tags
            .iter()
            .filter_map(|(id, tag)| tag.params.clone().map(|params| (id.0, params))),
    );
    let mut assignments: Vec<DefBodyAssign> = Vec::new();
    let operator_registry =
        OperatorRegistry::with_shared_signatures(&typed.defs, &ctx.cross_file_callable_signatures);

    let domain_declarations =
        TagDeclarations::overlay(&file_ast.tags, &ctx.compile_time_eval_ast.tags);
    let trait_methods =
        super::trait_methods::TraitMethods::new(file_ast, &ctx.compile_time_eval_ast)
            .with_reads(implementation_reads);
    for def_id in &def_ids {
        if typed
            .defs
            .get(def_id)
            .is_some_and(|bind| bind.requires_specialization())
        {
            continue;
        }
        let Some(bind) = file_ast.defs.get(def_id) else {
            continue;
        };
        let effective = trait_methods.effective(bind);
        let bind = effective.as_ref();
        let explicit_terminal_return = matches!(
            &bind.value,
            BindValue::Body { exprs, ret }
                if exprs.is_empty() && ret.values.len() == 1 && ret.postcondition.is_none()
                    && bind.named_results.is_empty()
        );
        let normalized;
        let bind = if explicit_terminal_return {
            normalized = {
                let mut bind = bind.clone();
                let BindValue::Body { ret, .. } = &bind.value else {
                    unreachable!()
                };
                bind.value = BindValue::Expr(Box::new(ret.values[0].clone()));
                bind
            };
            &normalized
        } else {
            bind
        };
        let projects_params = !trait_methods.contracts(bind).is_empty()
            && bind.params.as_ref().is_some_and(|params| params.values().any(|p|
                matches!(&p.kind, ParameterKind::Tagged(s) if matches!(s.value, Expr::IntegerDomain(_)))));
        let receiver_type = receiver_types.get(def_id).and_then(|r| r.as_ref());
        let mut method_tag_types = std::borrow::Cow::Borrowed(&tag_types);
        if let Some(receiver) = receiver_type {
            method_tag_types
                .to_mut()
                .insert(Intern::from_ref("Self"), receiver.clone());
        }
        let tag_types = method_tag_types;
        let empty_locals = HashSet::new();
        let locals: &HashSet<Intern<String>> = param_sets.get(def_id).unwrap_or(&empty_locals);

        let mut env = FlowState::default();
        if let Some(typed_bind) = typed.defs.get(def_id) {
            let params = typed_bind.params.clone();
            let groups = typed_bind.param_groups.clone();
            let conventions = typed_bind.param_conventions.clone();
            for (slot, ((name, ty), group)) in params.iter().zip(&groups).enumerate() {
                let projected = if projects_params {
                    bind.params
                        .as_ref()
                        .and_then(|params| params.get(name))
                        .and_then(|p| {
                            let ParameterKind::Tagged(surface) = &p.kind else {
                                return None;
                            };
                            if !matches!(surface.value, Expr::IntegerDomain(_)) {
                                return None;
                            }
                            Some(
                                typed
                                    .type_registry
                                    .integer_validity_for_type(ty.without_reference_wrappers())
                                    .cloned()
                                    .map(|validity| Ty::AnonymousInteger { validity })
                                    .unwrap_or(Ty::Opaque(Intern::from_ref(
                                        "<non-integer-domain>",
                                    ))),
                            )
                        })
                } else {
                    None
                };
                let ty = projected.as_ref().unwrap_or(ty);
                if projected.is_some()
                    && let Some(parameter) =
                        bind.params.as_ref().and_then(|params| params.get(name))
                    && let ParameterKind::Tagged(surface) = &parameter.kind
                    && let Some(diagnostic) = TypeEnv::new(&tag_types)
                        .with_registry(&typed.type_registry)
                        .with_tag_params(&tag_params)
                        .with_tag_decls(&file_ast.tags)
                        .integer_domain_diagnostic(surface, &typed.span_table)
                {
                    typed.declaration_flaws.push((surface.span_id, diagnostic));
                }
                env.types.insert(*name, ty.clone());
                let place = typed.add_place(crate::typed::TypedPlace {
                    binder: ast::BinderId::new(
                        typed.file_id.0,
                        ast::BinderOwner::Parameter {
                            definition: *def_id,
                            slot: slot as u32,
                        },
                    ),
                    name: *name,
                    parent: None,
                    projection: None,
                    mutable: matches!(
                        conventions.get(slot),
                        Some(ast::ParamConvention::Own | ast::ParamConvention::Mutate)
                    ),
                    explicit_contract: true,
                    ty: ty.clone(),
                });
                env.places.insert(*name, place);
                let target = group
                    .map(ReferenceTargetGroup::Param)
                    .unwrap_or(ReferenceTargetGroup::Local(place));
                env.set_target_group(*name, Some(ReferenceTargetSet::singleton(target)));
                let version = typed.add_place_version(
                    place,
                    crate::typed::PlaceVersionOrigin::Parameter,
                    ty.clone(),
                    None,
                );
                env.place_versions.insert(*name, version);
            }
        }

        let mut return_ty = bind_explicit_ty_in_scope(
            bind,
            &typed.type_registry,
            &tag_types,
            &tag_params,
            (&file_ast.tags).into(),
        );
        if !bind.anonymous_result_alternatives.is_empty()
            && let Some(typed_bind) = typed.defs.get(def_id)
        {
            return_ty = Some(typed_bind.return_type.clone());
        }
        if return_ty.is_none()
            && let Some(typed_bind) = typed.defs.get(def_id)
            && matches!(typed_bind.return_type, Ty::ResultFamily { .. })
        {
            return_ty = Some(typed_bind.return_type.clone());
        }
        if let Some(typed_bind) = typed.defs.get(def_id)
            && return_ty.is_some()
            && bind.is_method()
        {
            return_ty = Some(typed_bind.return_type.clone());
        }
        let mut force_expected_return_ty = bind.return_tag.is_some()
            || bind.return_refinement.is_some()
            || bind.return_type_name.is_some()
            || bind.type_annotation.is_some()
            || !bind.named_results.is_empty()
            || !bind.anonymous_result_alternatives.is_empty()
            || matches!(return_ty, Some(Ty::ResultFamily { .. }));
        if let Some(receiver_type) = receiver_type
            && return_ty.is_none()
            && let Some(receiver_surface) = bind.receiver_type_surface()
            && let Some(return_tag) = &bind.return_tag
        {
            let returns_receiver = match (&receiver_surface.value, &return_tag.value) {
                (ast::Expr::TagCall(receiver_call), ast::Expr::TagCall(return_call)) => {
                    receiver_call.name == return_call.name
                }
                (ast::Expr::AnonymousTag(receiver_name), ast::Expr::AnonymousTag(return_name)) => {
                    receiver_name == return_name
                }
                (ast::Expr::TagCall(receiver_call), ast::Expr::AnonymousTag(return_name)) => {
                    receiver_call.name == *return_name
                }
                _ => false,
            };
            let returns_self = matches!(
                &return_tag.value,
                ast::Expr::AnonymousTag(name) if name.as_str() == "Self"
            ) || matches!(
                &return_tag.value,
                ast::Expr::TagCall(call) if call.name.as_str() == "Self"
            );
            if returns_self || returns_receiver {
                return_ty = Some(receiver_type.clone());
                force_expected_return_ty = true;
            }
        }
        if return_ty.is_none()
            && let Some(receiver_type) = receiver_type
            && bind.receiver_type_surface().is_some()
            && matches!(bind.name.as_str(), "new")
        {
            let returns_self_in_body = match &bind.value {
                BindValue::Expr(typed_expr) => {
                    matches!(&typed_expr.value, ast::Expr::TagCall(call) if call.name.as_str() == "Self")
                }
                BindValue::Body { ret, .. } => {
                    ret.values.first().is_some_and(|ret_expr| {
                            matches!(&ret_expr.value, ast::Expr::TagCall(call) if call.name.as_str() == "Self")
                        })
                }
                _ => false,
            };
            if returns_self_in_body {
                return_ty = Some(receiver_type.clone());
                force_expected_return_ty = true;
            }
        }
        if return_ty.is_none() {
            let expr_ctx = ExprTypeResolverCtx {
                tag_types: &tag_types,
                variant_map: &variant_map,
                receiver_type,
                local_var_types: &env.types,
                fn_return_types: &typed.fn_return_types,
                type_registry: &typed.type_registry,
            };
            let inferred = match &bind.value {
                BindValue::Expr(typed_expr) => {
                    Some(resolve_expr_type(typed_expr.as_ref(), &expr_ctx, None))
                }
                BindValue::Body { ret, .. } => Some(if !ret.values.is_empty() {
                    let types = ret
                        .values
                        .iter()
                        .map(|typed_expr| resolve_expr_type(typed_expr, &expr_ctx, None))
                        .collect::<Vec<_>>();
                    if bind.named_results.len() == types.len() && types.len() > 1 {
                        Ty::Sequence(
                            bind.named_results
                                .iter()
                                .zip(types)
                                .map(|(result, ty)| ast::ty::SequenceTypeElement {
                                    name: result.name,
                                    ty: Box::new(ty),
                                })
                                .collect(),
                        )
                    } else if let [ty] = types.as_slice() {
                        ty.clone()
                    } else {
                        Ty::sequence(
                            types
                                .into_iter()
                                .map(|ty| ast::ty::SequenceTypeElement {
                                    name: None,
                                    ty: Box::new(ty),
                                })
                                .collect(),
                        )
                    }
                } else {
                    Ty::Unit
                }),
                _ => None,
            };
            if let Some(ty) = inferred {
                return_ty = Some(ty);
            }
        }
        let return_ty = return_ty;
        let is_block_body = matches!(
            &bind.value,
            BindValue::Expr(expr) if matches!(expr.value, Expr::Block(_))
        );
        let allows_fallthrough = is_block_body
            && (!force_expected_return_ty
                || return_ty
                    .as_ref()
                    .is_some_and(super::return_types::has_no_results));
        let scope = ExprLowerScope {
            tag_types: &tag_types,
            variant_map: &variant_map,
            receiver_type,
            expected_ty: force_expected_return_ty
                .then_some(return_ty.as_ref())
                .flatten(),
            return_ty: return_ty
                .as_ref()
                .filter(|ty| force_expected_return_ty || matches!(ty, Ty::ResultFamily { .. })),
            value_required: true,
            when_subject: None,
            locals,
            current_def_id: Some(*def_id),
            callable_signatures: &ctx.cross_file_callable_signatures,
            overload_groups: &ctx.overload_groups,
            operator_registry: &operator_registry,
            tag_params: &tag_params,
            tag_decls: domain_declarations,
            source_defs: &file_ast.defs,
            implementation_reads,
            constant_queries,
            allow_literal_materialization: true,
        };

        let proposition_parameters = typed
            .defs
            .get(def_id)
            .map(|bind| bind.params.to_vec())
            .unwrap_or_default();
        for (name, ty) in proposition_parameters {
            if let Some(evidence) = super::lower_when::parameter_proposition_evidence(
                typed,
                &ty,
                bind.name_span,
                &scope,
                &mut env,
            ) {
                env.locals.insert(name);
                env.result_evidence.insert(name, evidence);
            }
        }
        let mut body = match &bind.value {
            BindValue::Expr(typed_expr) => {
                let expression_scope = if allows_fallthrough {
                    scope.statement()
                } else {
                    scope.with_expected(scope.expected_ty)
                };
                let id = lower_typed_expr(typed, typed_expr.as_ref(), &expression_scope, &mut env);
                if let (true, Some(explicit)) = (force_expected_return_ty, return_ty.clone())
                    && !allows_fallthrough
                    && !matches!(explicit, Ty::UnresolvedLiteral(_))
                    && !matches!(
                        explicit,
                        Ty::Opaque(name) if name.as_str() == "unresolved-array-construction"
                    )
                {
                    if projects_params {
                        super::lower_ty::validate_projected_return(typed, bind, id, &explicit);
                    }
                    super::lower_ty::validate_domain_annotation(typed, bind, id, &explicit, None);
                    typed.exprs.ty[id.as_usize()] = explicit.clone();
                    retarget_literal_materialization(
                        &mut typed.exprs.facts.const_facts.literal_materialization[id.as_usize()],
                        &explicit,
                        &typed.type_registry,
                    );
                    let literal_values = typed
                        .type_registry
                        .resolved_definition_for_type(&explicit)
                        .union_literal_values()
                        .map(<[ast::ConstValue]>::to_vec);
                    if let Some(values) = literal_values {
                        annotate_literal_union_literal(typed, id, &explicit, &values);
                    } else if !matches!(
                        typed.exprs.facts.const_facts.const_value[id.as_usize()],
                        Some(
                            ast::ConstValue::Int(_)
                                | ast::ConstValue::ResultAlternative { .. }
                                | ast::ConstValue::Record { .. }
                                | ast::ConstValue::Text(_),
                        )
                    ) {
                        typed.exprs.facts.const_facts.const_value[id.as_usize()] = None;
                    }
                }
                BindBody::Expr(id)
            }
            BindValue::Body { exprs, ret } => {
                let mut lowered_exprs: Vec<ExprId> = Vec::new();
                for expr in exprs {
                    lowered_exprs.push(lower_typed_expr(typed, expr, &scope.statement(), &mut env));
                }
                let ret_id = if !ret.values.is_empty() {
                    let values = ret.values.clone();
                    let span_id = ret.span_id;
                    let ret_expr = if values.len() == 1 {
                        values.into_iter().next().expect("one return value")
                    } else {
                        Typed::infer(Expr::ResultValues(values), span_id)
                    };
                    Some(lower_typed_expr(typed, &ret_expr, &scope, &mut env))
                } else {
                    None
                };
                if let (Some(value), Some(expected)) = (ret_id, &return_ty) {
                    if projects_params {
                        super::lower_ty::validate_projected_return(typed, bind, value, expected);
                    }
                    super::lower_ty::validate_domain_annotation(typed, bind, value, expected, None);
                }
                BindBody::Body {
                    exprs: lowered_exprs,
                    ret: ret_id,
                }
            }
            BindValue::Unassigned => BindBody::Unlowered,
        };
        let falls_through = allows_fallthrough
            && matches!(&body, BindBody::Expr(expression)
                if !crate::analysis::expression_returns(typed.resolved_program(), *expression));
        let return_ty = if falls_through {
            let BindBody::Expr(expression) = body else {
                unreachable!()
            };
            super::return_types::validate_unit_fallthrough(typed, bind, expression);
            body = BindBody::Body {
                exprs: vec![expression],
                ret: None,
            };
            Some(Ty::Unit)
        } else {
            return_ty
        };
        if is_block_body {
            if let BindBody::Expr(expression) = &body {
                if !crate::analysis::expression_returns(typed.resolved_program(), *expression) {
                    // type-missing-return
                    typed.defs.get_mut(def_id).unwrap().push_flaw(flaw(
                        &typed.span_table,
                        bind.name_span,
                        DiagnosticCode::Type(TypeCode::Result(ResultCode::ReturnMissing)),
                        "block-bodied functions with results require an explicit return on every path",
                    ));
                }
            }
        }
        if let (Some(Ty::Sequence(fields)), BindValue::Body { exprs, ret }) =
            (return_ty.as_ref(), &bind.value)
        {
            let expected_fields = fields.clone();
            let expr_ctx = ExprTypeResolverCtx {
                tag_types: &tag_types,
                variant_map: &variant_map,
                receiver_type,
                local_var_types: &env.types,
                fn_return_types: &typed.fn_return_types,
                type_registry: &typed.type_registry,
            };
            let mut validate_values = |values: &[Typed<ast::Expr>]| {
                if values.is_empty() {
                    return;
                }
                if let [value] = values
                    && !matches!(value.value, Expr::SequenceLiteral(_))
                {
                    let actual = resolve_expr_type(value, &expr_ctx, None);
                    if let Ty::Sequence(actual_fields) = &actual {
                        let diagnostic = if actual_fields.len() != expected_fields.len() {
                            // type-return-arity
                            Some(Diagnostic::new(
                                ResultCode::ReturnArity,
                                format!(
                                    "returned {} values, but the callable declares {} results",
                                    actual_fields.len(),
                                    expected_fields.len()
                                ),
                            ))
                        } else if actual_fields != &expected_fields {
                            // type-return-mismatch
                            Some(Diagnostic::new(
                                ResultCode::ReturnMismatch,
                                "forwarded result row does not match the declared result row",
                            ))
                        } else {
                            None
                        };
                        if let Some(diagnostic) = diagnostic
                            && let Some(typed_bind) = typed.defs.get_mut(def_id)
                        {
                            typed_bind
                                .push_flaw(diagnostic.at_span_id(value.span_id, &typed.span_table));
                        }
                        return;
                    }
                }
                if values.len() != expected_fields.len() {
                    if let Some(typed_bind) = typed.defs.get_mut(def_id) {
                        typed_bind.push_flaw(
                            // type-return-arity
                            Diagnostic::new(
                                ResultCode::ReturnArity,
                                format!(
                                    "returned {} values, but the callable declares {} results",
                                    values.len(),
                                    expected_fields.len()
                                ),
                            )
                            .at_span_id(values[0].span_id, &typed.span_table),
                        );
                    }
                    return;
                }
                for (value, field) in values.iter().zip(&expected_fields) {
                    let actual = resolve_expr_type(value, &expr_ctx, None);
                    if !matches!(value.value, Expr::SequenceLiteral(_))
                        && !matches!(actual, Ty::UnresolvedLiteral(_) | Ty::Sequence(_))
                        && actual != *field.ty
                        && let Some(typed_bind) = typed.defs.get_mut(def_id)
                    {
                        // type-return-mismatch
                        typed_bind.push_flaw(flaw(
                            &typed.span_table,
                            value.span_id,
                            DiagnosticCode::Type(TypeCode::Result(ResultCode::ReturnMismatch)),
                            "returned value does not match its declared result type",
                        ));
                    }
                }
            };
            for expr in exprs {
                if let ast::Expr::If(if_expr) = &expr.value {
                    validate_values(&if_expr.ret.values);
                }
            }
            if let [value] = ret.values.as_slice() {
                let actual = resolve_expr_type(value, &expr_ctx, None);
                if matches!(actual, Ty::Sequence(_))
                    && !matches!(value.value, Expr::SequenceLiteral(_))
                    && actual != *return_ty.as_ref().unwrap()
                    && let Some(typed_bind) = typed.defs.get_mut(def_id)
                {
                    // type-return-mismatch
                    typed_bind.push_flaw(flaw(
                        &typed.span_table,
                        value.span_id,
                        DiagnosticCode::Type(TypeCode::Result(ResultCode::ReturnMismatch)),
                        "forwarded result row does not match the declared result row",
                    ));
                }
            }
            for (value, field) in ret.values.iter().zip(fields) {
                if ret.values.len() == 1 && matches!(value.value, Expr::SequenceLiteral(_)) {
                    continue;
                }
                let actual = resolve_expr_type(value, &expr_ctx, None);
                if !matches!(value.value, Expr::SequenceLiteral(_) | Expr::Todo)
                    && !matches!(actual, Ty::UnresolvedLiteral(_) | Ty::Sequence(_))
                    && actual != *field.ty
                    && let Some(typed_bind) = typed.defs.get_mut(def_id)
                {
                    // type-return-mismatch
                    typed_bind.push_flaw(flaw(
                        &typed.span_table,
                        value.span_id,
                        DiagnosticCode::Type(TypeCode::Result(ResultCode::ReturnMismatch)),
                        "returned value does not match its declared result type",
                    ));
                }
            }
        }
        if let Some(expected) = &return_ty {
            for value in conditional_return_expressions(typed, &body) {
                let Some(actual) = typed.exprs.ty_of(value).cloned() else {
                    continue;
                };
                let expected_count = expected
                    .sequence_len()
                    .unwrap_or(usize::from(!matches!(expected, Ty::Unit)));
                let actual_count = actual
                    .sequence_len()
                    .unwrap_or(usize::from(!matches!(actual, Ty::Unit)));
                let mut diagnostics = Vec::new();
                if actual_count != expected_count {
                    diagnostics.push((
                        match typed.exprs.kind_of(value) {
                            Some(TypedExprKind::ResultRow(values)) => {
                                values.first().copied().unwrap_or(value)
                            }
                            _ => value,
                        },
                        // type-return-arity
                        Diagnostic::new(
                            ResultCode::ReturnArity,
                            format!(
                                "returned {} values, but the callable declares {} results",
                                actual_count, expected_count
                            ),
                        ),
                    ));
                } else if let (Some(TypedExprKind::ResultRow(values)), Ty::Sequence(fields)) =
                    (typed.exprs.kind_of(value), expected)
                {
                    for (value, field) in values.iter().zip(fields) {
                        let actual = match typed.exprs.kind_of(*value) {
                            Some(TypedExprKind::FnCall {
                                substituted_ty: Some(actual),
                                ..
                            }) => Some(actual),
                            Some(TypedExprKind::Todo) => None,
                            _ => typed.exprs.ty_of(*value),
                        };
                        if actual.is_some_and(|actual| {
                            actual != field.ty.as_ref()
                                && !matches!(actual, Ty::UnresolvedLiteral(_))
                        }) {
                            diagnostics.push((
                                *value,
                                // type-return-mismatch
                                Diagnostic::new(
                                    ResultCode::ReturnMismatch,
                                    "returned value does not match its declared result type",
                                ),
                            ));
                        }
                    }
                } else if matches!((&actual, expected), (Ty::Sequence(_), Ty::Sequence(_)))
                    && actual != *expected
                {
                    diagnostics.push((
                        value,
                        // type-return-mismatch
                        Diagnostic::new(
                            ResultCode::ReturnMismatch,
                            "forwarded result row does not match the declared result row",
                        ),
                    ));
                } else if let (
                    Ty::Named {
                        instance: actual, ..
                    },
                    Ty::Named {
                        instance: expected, ..
                    },
                ) = (&actual, expected)
                    && (actual.declaration != expected.declaration
                        || (!named_instance_has_symbolic_argument(actual)
                            && actual.arguments != expected.arguments))
                {
                    diagnostics.push((
                        value,
                        // type-return-mismatch
                        Diagnostic::new(
                            ResultCode::ReturnMismatch,
                            "returned nominal type does not match the declared return type",
                        ),
                    ));
                }
                for (value, diagnostic) in diagnostics {
                    let span = typed.exprs.span_of(value).unwrap_or(bind.name_span);
                    typed
                        .defs
                        .get_mut(def_id)
                        .unwrap()
                        .push_flaw(diagnostic.at_span_id(span, &typed.span_table));
                }
            }
        }
        if (projects_params || bind.return_refinement.is_some())
            && let Some(expected) = &return_ty
        {
            let exits = conditional_return_expressions(typed, &body);
            for value in exits {
                super::lower_ty::validate_projected_return(typed, bind, value, expected);
            }
        }
        let return_expr_id = match &body {
            BindBody::Expr(expr)
            | BindBody::Body {
                ret: Some(expr), ..
            } => Some(*expr),
            BindBody::Body { ret: None, .. } => None,
            BindBody::Unlowered => None,
        };
        let infers_return_type = super::return_types::infers_return_type(bind);
        let inferred_integer_return_type = infers_return_type
            .then(|| super::return_types::infer_integer_return_type(typed, bind, &body))
            .flatten();
        if let Some(return_type) = &inferred_integer_return_type {
            super::return_types::validate_integer_return_representation(typed, bind, return_type);
        }
        let inferred_return_type = if let Some(return_type) = &inferred_integer_return_type {
            Some(return_type.clone())
        } else if infers_return_type {
            {
                match &body {
                    BindBody::Expr(expr)
                    | BindBody::Body {
                        ret: Some(expr), ..
                    } => typed.exprs.ty.get(expr.as_usize()).and_then(|expr_ty| {
                        infer_fixed_array_return_type(typed, *expr, expr_ty)
                            .or_else(|| Some(expr_ty.clone()))
                    }),
                    BindBody::Body { ret: None, .. } => Some(Ty::Unit),
                    BindBody::Unlowered => None,
                }
            }
        } else {
            None
        };
        if inferred_return_type
            .as_ref()
            .or(return_ty.as_ref())
            .is_some_and(|ty| !matches!(ty, Ty::Unit) && ty.sequence_len() != Some(0))
        {
            let expressions = match &bind.value {
                BindValue::Body { exprs, .. } => exprs.as_slice(),
                BindValue::Expr(expr) => std::slice::from_ref(expr.as_ref()),
                BindValue::Unassigned => &[],
            };
            for span in bare_conditional_returns(expressions) {
                if let Some(bind) = typed.defs.get_mut(def_id) {
                    // type-return-mismatch
                    bind.push_flaw(flaw(
                        &typed.span_table,
                        span,
                        DiagnosticCode::Type(TypeCode::Result(ResultCode::ReturnMismatch)),
                        "bare return requires a function with no results",
                    ));
                }
            }
        }
        let result_exit_owners = result_family_exit_owners(typed, &body);
        if result_exit_owners.len() > 1
            && let Some(typed_bind) = typed.defs.get_mut(def_id)
        {
            // type-result-family-mixed-owners
            typed_bind.push_flaw(flaw(
                &typed.span_table,
                bind.name_span,
                DiagnosticCode::Type(TypeCode::Result(ResultCode::FamilyMixedOwners)),
                "an inferred callable cannot combine anonymous result families with different owners",
            ));
        }
        if result_exit_owners
            .iter()
            .any(|owner| matches!(owner, ast::ResultFamilyOwner::LocalCallable(_)))
            && let Some(typed_bind) = typed.defs.get_mut(def_id)
        {
            // type-local-result-family-escapes
            typed_bind.push_flaw(flaw(
                &typed.span_table,
                bind.name_span,
                DiagnosticCode::Type(TypeCode::Result(ResultCode::LocalFamilyEscapes)),
                "a local callable result family cannot escape its enclosing callable",
            ));
        }
        if let (Some(return_expr_id), Some(return_type)) =
            (return_expr_id, inferred_return_type.as_ref())
            && inferred_integer_return_type.is_none()
            && let Some(stored) = typed.exprs.ty.get_mut(return_expr_id.as_usize())
            && stored != return_type
        {
            if let Some(typed_bind) = typed.defs.get_mut(def_id) {
                // type-return-mismatch
                typed_bind.push_flaw(flaw(
                    &typed.span_table,
                    bind.name_span,
                    DiagnosticCode::Type(TypeCode::Result(ResultCode::ReturnMismatch)),
                    "returned value does not match the declared return type",
                ));
            }
            *stored = return_type.clone();
            retarget_literal_materialization(
                &mut typed.exprs.facts.const_facts.literal_materialization
                    [return_expr_id.as_usize()],
                return_type,
                &typed.type_registry,
            );
        }
        if force_expected_return_ty
            && let Some(explicit_return_type) = return_ty.as_ref()
            && !matches!(
                explicit_return_type,
                Ty::Opaque(name) if name.as_str() == "unresolved-array-construction"
            )
        {
            typed
                .fn_return_types
                .insert(*def_id, explicit_return_type.clone());
            let supplied_count = match &bind.value {
                BindValue::Expr(_) if falls_through => 0,
                BindValue::Expr(_) => return_expr_id
                    .and_then(|id| typed.exprs.ty_of(id).and_then(Ty::sequence_len))
                    .unwrap_or(1),
                BindValue::Body { ret, .. } if ret.values.len() == 1 => return_expr_id
                    .and_then(|id| typed.exprs.ty_of(id).and_then(Ty::sequence_len))
                    .unwrap_or(1),
                BindValue::Body { ret, .. } if !ret.values.is_empty() => ret.values.len(),
                BindValue::Body { ret, .. } => usize::from(!ret.values.is_empty()),
                BindValue::Unassigned => 0,
            };
            let expected_count = explicit_return_type
                .sequence_len()
                .unwrap_or(usize::from(!matches!(explicit_return_type, Ty::Unit)));
            if !matches!(bind.value, BindValue::Unassigned)
                && supplied_count != expected_count
                && let Some(typed_bind) = typed.defs.get_mut(def_id)
            {
                typed_bind.push_flaw(
                    // type-return-arity
                    Diagnostic::new(
                        ResultCode::ReturnArity,
                        format!(
                            "returned {} values, but the callable declares {} results",
                            supplied_count, expected_count
                        ),
                    )
                    .at_span_id(bind.name_span, &typed.span_table),
                );
            }
            if let Some(return_expr_id) = return_expr_id
                && typed.exprs.ty_of(return_expr_id) != Some(explicit_return_type)
                && let Some(typed_bind) = typed.defs.get_mut(def_id)
                && !typed_bind.flaws.iter().any(|flaw| {
                    flaw.code == DiagnosticCode::Type(TypeCode::Result(ResultCode::ReturnMismatch))
                })
            {
                // type-return-mismatch
                typed_bind.push_flaw(flaw(
                    &typed.span_table,
                    bind.name_span,
                    DiagnosticCode::Type(TypeCode::Result(ResultCode::ReturnMismatch)),
                    "returned value does not match the declared return type",
                ));
            }
            if let Some(typed_bind) = typed.defs.get_mut(def_id) {
                typed_bind.return_type = explicit_return_type.clone();
            }
        }
        if let Some(return_type) = inferred_return_type.as_ref() {
            typed.fn_return_types.insert(*def_id, return_type.clone());
        }
        if let Some(typed_bind) = typed.defs.get_mut(def_id) {
            typed_bind.body = body.clone();
            typed_bind.postcondition = match &bind.value {
                BindValue::Body { ret, .. } => {
                    crate::typed::TypedPostcondition::from_return(ret, &env.types)
                }
                _ => None,
            }
            .map(Box::new);
            typed_bind.return_evidence = return_expr_id
                .and_then(|expr| typed.exprs.value_evidence_of(expr).cloned())
                .map(Box::new);
            if let Some(return_type) = inferred_return_type {
                typed_bind.return_type = return_type;
            }
        }
        super::return_types::validate_proposition_returns(typed, bind, &body);
        assignments.push(DefBodyAssign {
            def_id: *def_id,
            body,
        });
    }
    refresh_result_evidence_summaries(typed, def_ids.len().saturating_add(1));

    // Lower top-level expressions.
    let empty_locals = HashSet::new();
    let mut env = FlowState::default();
    let scope = ExprLowerScope {
        tag_types: &tag_types,
        variant_map: &variant_map,
        receiver_type: None,
        expected_ty: None,
        return_ty: None,
        value_required: false,
        when_subject: None,
        locals: &empty_locals,
        current_def_id: None,
        callable_signatures: &ctx.cross_file_callable_signatures,
        overload_groups: &ctx.overload_groups,
        operator_registry: &operator_registry,
        tag_params: &tag_params,
        tag_decls: domain_declarations,
        source_defs: &file_ast.defs,
        implementation_reads,
        constant_queries,
        allow_literal_materialization: true,
    };
    for (expr, span_id) in &file_ast.exprs {
        let wrapped = Typed::infer(expr.clone(), *span_id);
        let expr_id = lower_typed_expr(typed, &wrapped, &scope, &mut env);
        typed.root_exprs.push(expr_id);
    }

    // Check for redundant `self` parameter type annotations on methods.
    for def_id in &def_ids {
        let Some(bind) = file_ast.defs.get(def_id) else {
            continue;
        };
        if !bind.is_method() {
            continue;
        }
        if let Some(params) = &bind.params
            && params.iter().any(|(name, kind)| {
                name.as_str() == "self" && matches!(&kind.kind, ParameterKind::Tagged(surface) if !matches!(surface.value, Expr::IntegerDomain(_)))
            })
            && let Some(typed_bind) = typed.defs.get_mut(def_id)
        {
            // type-self-param-typed
            typed_bind.push_flaw(Diagnostic::new(
                CallCode::SelfParameterTyped,
                "self parameter should not have a type annotation",
            ));
        }
    }

    // Second pass: assign bodies to defs.
    for assign in assignments {
        if let Some(typed_bind) = typed.defs.get_mut(&assign.def_id) {
            typed_bind.body = assign.body;
        }
    }
    super::member_access::validate(typed);
    super::storage_contracts::validate(typed);
}

pub(super) fn conditional_return_expressions(typed: &TypedFileAst, body: &BindBody) -> Vec<ExprId> {
    let mut pending = match body {
        BindBody::Expr(expr) => vec![*expr],
        BindBody::Body { exprs, ret } => exprs.iter().copied().chain(*ret).collect(),
        BindBody::Unlowered => return Vec::new(),
    };
    pending.reverse();
    let mut exits = Vec::new();
    let mut visited = HashSet::new();
    while let Some(expr) = pending.pop() {
        if !visited.insert(expr) || typed.local_callable_parameters.contains_key(&expr) {
            continue;
        }
        match &typed.exprs.kind[expr.as_usize()] {
            TypedExprKind::Return { value, .. } => {
                if let Some(value) = value {
                    exits.extend(crate::analysis::return_expressions(
                        typed,
                        &BindBody::Expr(*value),
                    ));
                }
            }
            TypedExprKind::Block { statements, value } => {
                pending.extend(value.iter().copied());
                pending.extend(statements.iter().rev().copied());
            }
            TypedExprKind::If(branch) => {
                if let Some(ret) = branch.ret {
                    exits.push(ret);
                    pending.push(ret);
                }
                pending.extend(branch.stmts.iter().rev().copied());
            }
            TypedExprKind::Loop(body) => {
                for batch in body.statement_batches().rev() {
                    pending.extend(batch.iter().rev().copied());
                }
            }
            TypedExprKind::When(branch) => {
                pending.extend(branch.arms.iter().rev().map(|arm| match arm {
                    TypedWhenArm::Cond { body, .. }
                    | TypedWhenArm::Is { body, .. }
                    | TypedWhenArm::Else(body, _) => *body,
                }));
            }
            TypedExprKind::Bind {
                stmts,
                body,
                unassigned: false,
                ..
            } => {
                pending.push(*body);
                pending.extend(stmts.iter().rev().copied());
            }
            _ => {}
        }
    }
    exits
}

fn bare_conditional_returns(exprs: &[Typed<Expr>]) -> Vec<SpanId> {
    let mut spans = Vec::new();
    let mut pending: Vec<_> = exprs.iter().collect();
    while let Some(expr) = pending.pop() {
        if matches!(&expr.value, Expr::Bind(bind) if bind.params.is_some()) {
            continue;
        }
        if let Expr::Return(ret) = &expr.value {
            if ret.values.is_empty() {
                spans.push(ret.span_id);
            }
        }
        let _ = ast::folder::walk_typed_expr_children(&expr.value, &mut |child| {
            pending.push(child);
            std::ops::ControlFlow::Continue(())
        });
    }
    spans
}

fn result_family_exit_owners(
    typed: &TypedFileAst,
    body: &BindBody,
) -> HashSet<ast::ResultFamilyOwner> {
    let mut owners = HashSet::new();
    for expression in crate::analysis::return_expressions(typed, body) {
        let ty = match typed.exprs.kind_of(expression) {
            Some(TypedExprKind::FnCall { target, .. }) => typed
                .defs
                .get(target)
                .map(|bind| &bind.return_type)
                .or_else(|| typed.fn_return_types.get(target))
                .unwrap_or(&typed.exprs.ty[expression.index()]),
            _ => &typed.exprs.ty[expression.index()],
        };
        if let Ty::ResultFamily { owner, .. } = ty {
            owners.insert(owner.clone());
        }
    }
    owners
}

fn refresh_result_evidence_summaries(typed: &mut TypedFileAst, max_passes: usize) {
    for _ in 0..max_passes {
        let mut changed = false;
        for index in 0..typed.exprs.kind.len() {
            let kind = typed.exprs.kind[index].clone();
            let ty = typed.exprs.ty[index].clone();
            let evidence = match &kind {
                TypedExprKind::Bind { body, .. }
                | TypedExprKind::Block {
                    value: Some(body), ..
                } => typed.exprs.value_evidence_of(*body).cloned(),
                _ => value_evidence_for(&kind, &ty, typed),
            }
            .map(Box::new);
            if evidence.is_some()
                && typed.exprs.facts.semantic_facts.result_evidence[index] != evidence
            {
                typed.exprs.facts.semantic_facts.result_evidence[index] = evidence;
                changed = true;
            }
        }
        for bind in typed.defs.values_mut() {
            let returned = match &bind.body {
                BindBody::Expr(expr)
                | BindBody::Body {
                    ret: Some(expr), ..
                } => typed.exprs.value_evidence_of(*expr).cloned().map(Box::new),
                BindBody::Body { ret: None, .. } | BindBody::Unlowered => None,
            };
            if returned.is_some() && bind.return_evidence != returned {
                bind.return_evidence = returned;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn infer_fixed_array_return_type(
    typed: &TypedFileAst,
    expr_id: ExprId,
    expr_ty: &Ty,
) -> Option<Ty> {
    let typed_expr = typed.exprs.kind.get(expr_id.as_usize())?;
    let TypedExprKind::TagCall {
        args: Some(args), ..
    } = typed_expr
    else {
        return None;
    };
    let arg_element_expr = args.first()?;
    let element_ty = typed.exprs.ty.get(arg_element_expr.as_usize())?.clone();
    let arg_len_expr = args.get(1)?;
    let Some(ast::ConstValue::Int(length)) = typed
        .exprs
        .facts
        .const_facts
        .const_value
        .get(arg_len_expr.as_usize())?
        .as_ref()
    else {
        return None;
    };
    let length = ast::integer::to_u64(*length)?;

    let Ty::Named { instance, name, .. } = expr_ty else {
        return None;
    };
    let declaration = typed.type_registry.declaration_for_instance(instance)?;
    let former = declaration.sequence_literal.as_ref()?;
    let length_parameter = former.fixed_array_length_parameter()?;

    let mut instance = instance.clone();
    let mut saw_length = false;
    let mut saw_element = false;
    for (parameter, argument) in &mut instance.arguments {
        if *parameter == length_parameter {
            *argument = ast::TyArg::Const(ast::NormalExpr::from(length as i128));
            saw_length = true;
        } else if *parameter == former.element_parameter {
            *argument = ast::TyArg::Type(Box::new(element_ty.clone()));
            saw_element = true;
        }
    }
    if !saw_length || !saw_element {
        return None;
    }

    Some(Ty::Named {
        instance,
        name: *name,
    })
}

pub(crate) fn lower_typed_expr(
    typed: &mut TypedFileAst,
    expr: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> ExprId {
    if let Some((span, subject)) = scope.when_subject {
        if span == expr.span_id {
            return subject;
        }
    }
    if let Expr::ResultValues(values) = &expr.value
        && matches!(scope.expected_ty, Some(Ty::Array { .. }))
    {
        let literal = Typed::infer(
            Expr::SequenceLiteral(ast::SequenceLiteral::Elements(
                values
                    .iter()
                    .cloned()
                    .map(ast::SequenceElement::new)
                    .collect(),
            )),
            expr.span_id,
        );
        return lower_typed_expr(typed, &literal, scope, env);
    }
    if matches!(
        expr.value,
        Expr::DomainOf(_) | Expr::TagCall(_) | Expr::AnonymousTag(_) | Expr::RecordGet { .. }
    ) || matches!(&expr.value, Expr::FnCall(call) if !call.path.segments.is_empty())
    {
        let mut domain_types = std::borrow::Cow::Borrowed(scope.tag_types);
        if !env.places.is_empty() {
            domain_types.to_mut().extend(
                env.places
                    .iter()
                    .map(|(name, place)| (*name, typed.places[place.0 as usize].ty.clone())),
            );
        }
        let types = TypeEnv::new(&domain_types)
            .with_registry(&typed.type_registry)
            .with_tag_params(scope.tag_params)
            .with_tag_declaration_lookup(scope.tag_decls);
        let domains = crate::domain::Domains::new(types, scope.tag_decls)
            .with_values(&env.const_values)
            .with_file(&typed.eval_ast)
            .with_definitions(scope.source_defs)
            .with_reads(scope.implementation_reads);
        match domains.value(&expr.value) {
            Ok(Some(value)) => {
                let rewritten = crate::const_value::const_value_to_typed_expr(&value, expr.span_id);
                return lower_typed_expr(typed, &rewritten, scope, env);
            }
            Err(diagnostic) => {
                let invalid = Typed::infer(Expr::Invalid(expr.span_id), expr.span_id);
                let id = lower_typed_expr(typed, &invalid, scope, env);
                typed.exprs.facts.semantic_facts.flaws[id.as_usize()]
                    .push(diagnostic.at_span_id(expr.span_id, &typed.span_table));
                return id;
            }
            Ok(None) => {
                if let Ok(Some(projected)) = domains.projection(&expr.value) {
                    let rewritten = Typed::infer(projected, expr.span_id);
                    return lower_typed_expr(typed, &rewritten, scope, env);
                }
            }
        }
    }
    if let Expr::FnCall(call) = &expr.value
        && call.args.is_none()
        && call.path.segments.is_empty()
        && (scope.expected_ty.is_none() || scope.expected_ty != env.types.get(&call.path.root))
        && let Some(version) = env.place_versions.get(&call.path.root).copied()
        && let Some((variant, ty)) = env.union_members.remove(&version)
    {
        let source = lower_typed_expr(typed, expr, &scope.child(), env);
        env.union_members.insert(version, (variant, ty.clone()));
        return append_derived_expr(
            typed,
            source,
            TypedExprKind::UnionPayload {
                expr: source,
                variant,
            },
            ty,
        );
    }
    if let Some(Ty::Sequence(fields)) = scope.expected_ty
        && fields.len() == 1
        && fields[0].name.is_some()
        && !matches!(
            expr.value,
            Expr::SequenceLiteral(ast::SequenceLiteral::Elements(_))
                | Expr::ResultValues(_)
                | Expr::Bind(_)
                | Expr::Emit { .. }
                | Expr::Todo
        )
    {
        let actual = resolve_expr_type(
            expr,
            &ExprTypeResolverCtx {
                tag_types: scope.tag_types,
                variant_map: scope.variant_map,
                receiver_type: scope.receiver_type,
                local_var_types: &env.types,
                fn_return_types: &typed.fn_return_types,
                type_registry: &typed.type_registry,
            },
            None,
        );
        if !matches!(actual, Ty::Sequence(_)) {
            let wrapped = Typed::infer(
                Expr::SequenceLiteral(ast::SequenceLiteral::Elements(vec![
                    ast::SequenceElement::new(expr.clone()),
                ])),
                expr.span_id,
            );
            return lower_typed_expr(typed, &wrapped, scope, env);
        }
    }
    let mut resolved_ty = resolve_expr_type(
        expr,
        &ExprTypeResolverCtx {
            tag_types: scope.tag_types,
            variant_map: scope.variant_map,
            receiver_type: scope.receiver_type,
            local_var_types: &env.types,
            fn_return_types: &typed.fn_return_types,
            type_registry: &typed.type_registry,
        },
        scope.expected_ty,
    );
    if matches!(resolved_ty, Ty::UnresolvedLiteral(_))
        && let Some(expected) = scope.expected_ty
        && expected.is_int()
    {
        resolved_ty = expected.clone();
    }
    // Infer const_value from literals and known value refs at parse time (before const-folding runs).
    let mut const_val = expr
        .const_value
        .as_deref()
        .cloned()
        .or_else(|| match &expr.value {
            Expr::FnCall(call) if call.args.is_none() => {
                if call.path.value.segments.is_empty()
                    && let Some(cv) = env.const_values.get(&call.path.value.root)
                {
                    Some(cv.clone())
                } else if call.path.value.segments.is_empty()
                    && (env.types.contains_key(&call.path.value.root)
                        || scope.locals.contains(&call.path.value.root))
                {
                    None
                } else {
                    let target =
                        super::lower_ty::resolve_fn_call_target(&call.path.value, scope.tag_types);
                    const_for_def_id(
                        target,
                        typed,
                        scope.constant_queries,
                        scope.implementation_reads,
                    )
                    .or_else(|| {
                        scope.variant_map.get(&target.0).and_then(|entries| {
                            if entries.len() == 1 {
                                let qual_path =
                                    (!call.path.value.segments.is_empty()).then(|| {
                                        let mut path = call.path.value.root.as_str().to_string();
                                        for segment in
                                            call.path.value.segments.iter().take(
                                                call.path.value.segments.len().saturating_sub(1),
                                            )
                                        {
                                            path.push('.');
                                            path.push_str(segment.as_str());
                                        }
                                        path
                                    });
                                Some(ast::ConstValue::Tag {
                                    name: target.0,
                                    qual_path,
                                    args: vec![].into(),
                                })
                            } else {
                                None
                            }
                        })
                    })
                }
            }
            Expr::AnonymousTag(name) => scope.variant_map.get(name).and_then(|entries| {
                if entries.len() == 1 {
                    Some(ast::ConstValue::Tag {
                        name: *name,
                        qual_path: None,
                        args: vec![].into(),
                    })
                } else {
                    None
                }
            }),
            Expr::Lit(lit) => Some(crate::const_value::const_value_of_literal(lit)),
            Expr::Bind(bind) => match &bind.value {
                // For `x := 1`, look at the inner literal directly.
                ast::BindValue::Expr(inner) => match &inner.value {
                    Expr::Lit(lit) => Some(crate::const_value::const_value_of_literal(lit)),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        });
    let mut anonymous_intrinsic_operand_call = false;
    if let Some(qual_path) = source_const_qualifier(&expr.value)
        && let Some(ast::ConstValue::Tag {
            qual_path: stored, ..
        }) = const_val.as_mut()
        && stored.is_none()
    {
        *stored = Some(qual_path);
    }
    if let Expr::Negate(inner) = &expr.value
        && matches!(inner.value, Expr::Lit(Literal::Int(_)))
        && let Some(ast::ConstValue::Int(value)) = inner.const_value.as_deref().cloned().or({
            match &inner.value {
                Expr::Lit(Literal::Int(value)) => Some(ast::ConstValue::Int(*value)),
                _ => None,
            }
        })
    {
        const_val = Some(ast::ConstValue::Int(-value));
    }

    // Extract the span of the symbol name from the AST before lowering,
    // so diagnostics on unknown symbols point at the exact name in source.
    let name_span_id = match &expr.value {
        Expr::FnCall(call) => Some(call.path.span_id),
        Expr::TagCall(tc) => {
            // Qualified tag (e.g. `Maybe.Some`) — use the path span.
            if let Some(qp) = tc.qual_path.as_ref() {
                Some(qp.span_id)
            } else {
                // Bare tag (e.g. `Marker` in `Marker(x)`) — compute a span
                // covering just the tag name from the expression start.
                let expr_span = typed.span_table.get(expr.span_id);
                let name_len = tc.name.as_str().len();
                let span = Span::new(expr_span.start(), expr_span.start() + name_len);
                Some(typed.span_table.insert(span))
            }
        }
        _ => None,
    };

    let reserved_expr_id = matches!(
        expr.value,
        Expr::FnCall(_) | Expr::Bind(_) | Expr::RecordGet { .. }
    )
    .then(|| {
        typed.exprs.push_placeholder(
            TypedExprKind::Lit(Literal::Int(0.into())),
            Ty::Unit,
            expr.span_id,
        )
    });
    if matches!(expr.value, Expr::Loop(_)) {
        env.integer_domains.clear();
        env.union_members.clear();
        env.invalidate_mutable_constants();
    }
    let mut kind = lower_expr_kind(typed, expr, scope, env, reserved_expr_id);
    if let TypedExprKind::TargetQuery {
        kind: query,
        operand,
    } = &kind
    {
        let value = typed
            .target_layout
            .as_ref()
            .and_then(|layout| {
                layout
                    .query(*query, operand, Some(&typed.type_registry))
                    .ok()
            })
            .or_else(|| super::flow::invariant_layout_query(*query, operand, &typed.type_registry));
        if let Some(value) = value {
            let value = i256::I256::from(value);
            resolved_ty = Ty::AnonymousInteger {
                validity: ast::integer::IntegerValidity::new(
                    ast::integer::IntegerDomain::bounded(value, value).unwrap(),
                ),
            };
            const_val = Some(ConstValue::Int(value));
            kind = TypedExprKind::Lit(Literal::Int(value));
        } else if let Expr::TargetQuery { operand, .. } = &expr.value {
            let (min, max) = super::flow::layout_query_bounds(*query);
            resolved_ty = Ty::AnonymousInteger {
                validity: ast::integer::IntegerValidity::new(
                    ast::integer::IntegerDomain::from_refinement(&ast::PredicateExpr::And(vec![
                        ast::PredicateExpr::Ge(i256::I256::from(min).into()),
                        ast::PredicateExpr::Le(i256::I256::from(max).into()),
                        ast::PredicateExpr::Eq(ast::NormalExpr::TargetQuery {
                            kind: *query,
                            operand: operand.clone(),
                        }),
                    ])),
                ),
            };
        }
    }
    if let TypedExprKind::RecordSet { value, .. }
    | TypedExprKind::AggregateSet { value, .. }
    | TypedExprKind::BufSet { value, .. } = &kind
        && let Some(ty) = typed.exprs.ty_of(*value)
    {
        resolved_ty = ty.clone();
    }
    if matches!(
        &kind,
        TypedExprKind::Reassign { .. }
            | TypedExprKind::RecordSet { .. }
            | TypedExprKind::AggregateSet { .. }
            | TypedExprKind::BufSet { .. }
    ) {
        env.integer_domains.clear();
        env.union_members.clear();
        env.invalidate_mutable_constants();
        if matches!(&kind, TypedExprKind::Reassign { .. }) {
            env.projected_versions.clear();
        }
    }
    if let Some(callable) =
        reserved_expr_id.and_then(|call| typed.local_callable_calls.get(&call).copied())
    {
        let writes = super::local_call_effects::written_places(typed, callable, |target| {
            typed
                .defs
                .get(&target)
                .map(TypedCallableSignature::from)
                .or_else(|| scope.callable_signatures.get(&target).cloned())
                .is_none_or(|signature| super::local_call_effects::may_write(&signature))
        });
        let call = reserved_expr_id.expect("local calls reserve an expression ID");
        if let Some(writes) = writes {
            let mut affected: Vec<_> = env
                .places
                .iter()
                .filter(|(_, place)| writes.contains(place))
                .map(|(name, place)| (*name, *place))
                .collect();
            affected.sort_by_key(|(_, place)| place.0);
            for (name, place) in affected {
                env.const_values.remove(&name);
                if let Some(version) = env.place_versions.get(&name) {
                    env.integer_domains.remove(version);
                    env.union_members.remove(version);
                }
                let version = typed.add_place_version(
                    place,
                    crate::typed::PlaceVersionOrigin::Mutation(call),
                    typed.places[place.0 as usize].ty.clone(),
                    None,
                );
                env.set_place_version(name, version);
            }
            if !writes.is_empty() {
                env.projected_versions.clear();
            }
        } else {
            env.invalidate_value_evidence(typed, call);
        }
    } else if let TypedExprKind::FnCall { target, .. } = &kind
        && !env.places.contains_key(&target.0)
    {
        let conventions = typed
            .defs
            .get(target)
            .map(|bind| &bind.param_conventions)
            .or_else(|| {
                scope
                    .callable_signatures
                    .get(target)
                    .map(|signature| &signature.param_conventions)
            });
        if conventions.is_none_or(|conventions| conventions.contains(&ast::ParamConvention::Mutate))
        {
            let call = reserved_expr_id.unwrap_or(ExprId(typed.exprs.kind.len() as u32));
            env.invalidate_value_evidence(typed, call);
        }
    }

    // A named result projection is parsed as a record-style member access, then
    // lowered to AggregateGet once the callable result label resolves. Its
    // source-level type resolver cannot recover that label from structural
    // Recover the selected aggregate field type from the lowered base.
    if let TypedExprKind::AggregateGet { base, index } = kind
        && let Some(ty) = super::symbolic_values::projected_type(typed, base, index)
    {
        resolved_ty = ty;
    }

    if let TypedExprKind::SliceGet { buf, length, .. } = &kind {
        resolved_ty = slice_type(typed, *buf, *length);
    }
    if let TypedExprKind::Ref(inner) = &kind
        && let Expr::Ref { mutable, .. } = &expr.value
        && matches!(
            typed.exprs.kind_of(*inner),
            Some(TypedExprKind::SliceGet { .. })
        )
    {
        resolved_ty = Ty::Ref {
            inner: Box::new(typed.exprs.ty[inner.index()].clone()),
            mutable: *mutable,
        };
    }

    let mut arithmetic_diagnostic = None;
    if let TypedExprKind::IntrinsicCall { op, args } = &kind
        && let [left, right] = args.as_slice()
    {
        let operands = [left, right].map(|operand| arithmetic_operand_type(typed, *operand));
        if let Some(result) = crate::anonymous_arithmetic::result_type(
            *op,
            [&operands[0], &operands[1]],
            [
                typed.exprs.facts.const_facts.const_value[left.as_usize()].as_ref(),
                typed.exprs.facts.const_facts.const_value[right.as_usize()].as_ref(),
            ],
        ) {
            match result {
                Ok(ty) => resolved_ty = ty,
                Err(diagnostic) => arithmetic_diagnostic = Some(diagnostic),
            }
        }
    }

    if let TypedExprKind::IntrinsicCall {
        op: IntrinsicOp::BitsCompare { .. },
        ..
    } = &kind
    {
        if let Expr::Binary(binary) = &expr.value {
            resolved_ty =
                crate::analysis::propositions::comparison_type(binary).expect("numeric comparison");
        }
    }

    if matches!(
        resolved_ty,
        Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
    ) && let TypedExprKind::IntrinsicCall { args, .. } = &kind
        && args.len() == 2
    {
        let lhs = &typed.exprs.ty[args[0].as_usize()];
        let rhs = &typed.exprs.ty[args[1].as_usize()];

        let width = lhs
            .anonymous_integer_width()
            .into_iter()
            .chain(rhs.anonymous_integer_width())
            .max()
            .unwrap_or(64);
        let signed = matches!(
            lhs.anonymous_operation_interpretation()
                .or_else(|| rhs.anonymous_operation_interpretation()),
            Some(IntegerInterpretation::Signed)
        );
        resolved_ty = Ty::anonymous_integer_for_width(width, signed);
    }

    // For compile-time resolved when expressions, propagate the body's const_value.
    let mut const_val = if let TypedExprKind::When(ref when_expr) = kind {
        if when_expr.arms.len() == 1 && when_expr.subject.is_some() {
            let body_id = match &when_expr.arms[0] {
                TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. } => *body,
                TypedWhenArm::Else(body, _) => *body,
            };
            typed.exprs.facts.const_facts.const_value[body_id.as_usize()]
                .clone()
                .or(const_val)
        } else {
            const_val
        }
    } else {
        const_val
    };
    if const_val.is_none()
        && let TypedExprKind::TagCall {
            variant_id, args, ..
        } = &kind
        && args.as_ref().is_none_or(Vec::is_empty)
        && typed
            .type_registry
            .resolved_definition_for_type(&resolved_ty)
            .union_discriminant_domain()
            .is_some()
    {
        const_val = Some(ConstValue::Tag {
            name: variant_id.name,
            qual_path: Some(variant_id.union.0.as_str().to_owned()),
            args: Vec::new().into(),
        });
    }
    if let TypedExprKind::Cast { expr, ty } = &kind
        && let Some(value) = typed.exprs.const_value_of(*expr).and_then(Option::as_ref)
        && let Some(value) = crate::discriminant::cast_constant(
            &typed.type_registry,
            &typed.exprs.ty[expr.index()],
            ty,
            value,
        )
    {
        const_val = Some(value);
    }
    if const_val.is_none()
        && let TypedExprKind::IntrinsicCall {
            op: IntrinsicOp::BitsCompare { predicate, .. },
            args,
        } = &kind
        && let [left, right] = args.as_slice()
        && let (Some(ConstValue::Int(left)), Some(ConstValue::Int(right))) = (
            typed.exprs.facts.const_facts.const_value[left.as_usize()].as_ref(),
            typed.exprs.facts.const_facts.const_value[right.as_usize()].as_ref(),
        )
    {
        const_val = Some(ConstValue::Proposition(predicate.compare(*left, *right)));
    }
    if const_val.is_none()
        && let Ty::ResultFamily { alternatives, .. } = &resolved_ty
        && let TypedExprKind::TagCall { variant_id, .. } = &kind
        && alternatives
            .iter()
            .any(|alternative| alternative.label == variant_id.name)
    {
        const_val = Some(ConstValue::Tag {
            name: variant_id.name,
            qual_path: None,
            args: Vec::new().into(),
        });
    }
    if let Ty::ResultFamily {
        owner,
        alternatives,
    } = &resolved_ty
        && let Some(label) = const_val.as_ref().and_then(|value| match value {
            ConstValue::Tag { name, .. }
                if alternatives
                    .iter()
                    .any(|alternative| alternative.label == *name) =>
            {
                Some(*name)
            }
            _ => None,
        })
    {
        const_val = Some(ConstValue::ResultAlternative {
            owner: owner.clone(),
            label,
        });
    }

    let mut flaws: Vec<Diagnostic> = Vec::new();
    flaws.extend(arithmetic_diagnostic);

    if let (
        Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elems)),
        TypedExprKind::ResultRow(ids)
        | TypedExprKind::RecordValue(ids)
        | TypedExprKind::ArrayLit(ids),
    ) = (&expr.value, &kind)
        && let Some(expected) = scope.expected_ty
    {
        let collection = expected
            .named_instance()
            .and_then(|instance| typed.type_registry.declaration_for_instance(instance))
            .is_some_and(|declaration| declaration.sequence_literal.is_some());
        let definition = typed.type_registry.resolved_definition_for_type(expected);
        let fields = match &definition {
            Ty::Sequence(fields) => Some(fields.clone()),
            Ty::Record { fields, .. } if !collection => Some(
                fields
                    .iter()
                    .map(|(name, ty)| ast::ty::SequenceTypeElement {
                        name: Some(*name),
                        ty: ty.clone(),
                    })
                    .collect(),
            ),
            _ => None,
        };
        if let Some(fields) = fields {
            if elems.len() != fields.len() {
                flaws.push(
                    // type-sequence-arity
                    Diagnostic::new(
                        SequenceCode::Arity,
                        format!("expected {} elements, found {}", fields.len(), elems.len()),
                    )
                    .at_span_id(expr.span_id, &typed.span_table),
                );
            }
            for ((elem, id), field) in elems.iter().zip(ids).zip(&fields) {
                if elem.name.is_some() && elem.name != field.name {
                    // type-sequence-name-mismatch
                    flaws.push(flaw(
                        &typed.span_table,
                        elem.span_id,
                        DiagnosticCode::Type(TypeCode::Sequence(SequenceCode::NameMismatch)),
                        "sequence element name does not match the expected name at this position",
                    ));
                }
                let actual = &typed.exprs.ty[id.as_usize()];
                let array_reference_field = matches!(
                    typed.type_registry.resolved_definition_for_type(expected),
                    Ty::Record { .. }
                ) && field.ty.is_ref()
                    && actual.is_ref()
                    && (!field.ty.is_mutable_ref() || actual.is_mutable_ref())
                    && matches!(
                        typed
                            .type_registry
                            .resolved_definition_for_type(field.ty.without_reference_wrappers()),
                        Ty::Array { .. }
                    );
                if actual != field.ty.as_ref()
                    && !matches!(actual, Ty::UnresolvedLiteral(_))
                    && !array_reference_field
                {
                    // type-sequence-element-mismatch
                    flaws.push(flaw(
                        &typed.span_table,
                        elem.span_id,
                        DiagnosticCode::Type(TypeCode::Sequence(SequenceCode::ElementMismatch)),
                        "sequence element does not match its expected type",
                    ));
                }
            }
        } else if elems.iter().any(|elem| elem.name.is_some())
            && crate::representation::Repr::array_parts(expected, Some(&typed.type_registry))
                .is_some()
        {
            // type-sequence-name-mismatch
            flaws.push(flaw(
                &typed.span_table,
                expr.span_id,
                DiagnosticCode::Type(TypeCode::Sequence(SequenceCode::NameMismatch)),
                "named elements do not match an unnamed array type",
            ));
        }
    }
    if let TypedExprKind::AggregateGet { base, index }
    | TypedExprKind::AggregateSet { base, index, .. } = &kind
    {
        let ty = typed.exprs.ty_of(*base).unwrap();
        let ty = reference_pointee_for_type(ty, Some(&typed.type_registry))
            .unwrap_or_else(|| ty.clone());
        let len = match &ty {
            Ty::Sequence(fields) => Some(fields.len()),
            Ty::Record { fields, .. } => Some(fields.len()),
            _ => crate::representation::Repr::array_parts(&ty, Some(&typed.type_registry))
                .and_then(|(_, length)| usize::try_from(length).ok()),
        };
        if len.is_some_and(|len| *index >= len) {
            // type-sequence-index-out-of-bounds
            flaws.push(flaw(
                &typed.span_table,
                expr.span_id,
                DiagnosticCode::Type(TypeCode::Sequence(SequenceCode::IndexOutOfBounds)),
                "sequence element index is out of bounds",
            ));
        }
    }
    if let TypedExprKind::BufGet { buf, index } | TypedExprKind::BufSet { buf, index, .. } = &kind
        && let Some(ty) = typed.exprs.ty_of(*buf)
        && let Some(length) = crate::representation::Repr::array_parts(
            &reference_pointee_for_type(ty, Some(&typed.type_registry))
                .unwrap_or_else(|| ty.clone()),
            Some(&typed.type_registry),
        )
        .map(|(_, length)| length)
        .or_else(|| typed.type_registry.scalar_reference_extent(ty))
        && let Some(ConstValue::Int(index)) =
            typed.exprs.const_value_of(*index).and_then(Option::as_ref)
        && (*index < i256::I256::from(0) || *index >= i256::I256::from(length))
    {
        flaws.push(
            // type-array-index-out-of-bounds
            Diagnostic::new(
                ArrayCode::IndexOutOfBounds,
                format!("array index {index} is outside the extent {length}"),
            )
            .at_span_id(expr.span_id, &typed.span_table),
        );
    }
    if let TypedExprKind::BufGet { buf, .. } | TypedExprKind::BufSet { buf, .. } = &kind
        && let Some(Ty::Sequence(fields)) = typed.exprs.ty_of(*buf)
        && fields
            .first()
            .is_none_or(|first| fields.iter().any(|field| field.ty != first.ty))
    {
        // type-sequence-dynamic-index
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Sequence(SequenceCode::DynamicIndex)),
            "runtime indexing requires a common element type; use a constant position for this sequence",
        ));
    }
    if let TypedExprKind::TagCall {
        field_names,
        args: Some(args),
        ..
    } = &kind
    {
        match super::lower_tag::infer_record_arguments(&resolved_ty, field_names, args, typed) {
            Ok(Some(inferred)) => resolved_ty = inferred,
            Ok(None) => {}
            // type-record-type-arguments
            Err(message) => flaws.push(Diagnostic::new(GenericCode::RecordArguments, message)),
        }
    }

    if matches!(expr.value, Expr::FormatString(_)) {
        flaws.push(
            // type-template-strings-not-supported
            Diagnostic::new(
                TextCode::TemplatesUnsupported,
                "template strings are not supported yet",
            )
            .with_help("use ordinary text literals and explicit library calls"),
        );
    }
    if scope.expected_ty.is_some_and(Ty::is_mutable_ref)
        && let TypedExprKind::AggregateGet { base, index } = &kind
        && super::flow::field_has_array_dependency(typed, &typed.exprs.ty[base.index()], *index)
    {
        // type-dependent-array-mutation
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Array(ArrayCode::DependentMutation)),
            "cannot borrow an array's length or backing storage independently for mutation",
        ));
    }
    if let TypedExprKind::BufSet { buf, .. } = &kind
        && typed.call_argument_is_shared(*buf)
    {
        // type-mutable-borrow-from-shared
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Ownership(OwnershipCode::MutableBorrowFromShared)),
            "shared array storage cannot be mutated",
        ));
    }
    if let TypedExprKind::Ref(inner) = &kind
        && matches!(&expr.value, Expr::Ref { mutable: true, .. })
        && matches!(
            typed.exprs.kind_of(*inner),
            Some(TypedExprKind::BufGet { .. } | TypedExprKind::Deref(_))
        )
        && typed.call_argument_is_shared(*inner)
    {
        // type-mutable-borrow-from-shared
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Ownership(OwnershipCode::MutableBorrowFromShared)),
            "shared storage cannot supply a mutable element reference",
        ));
    }
    if let (
        Expr::SequenceLiteral(ast::SequenceLiteral::Elements(_)),
        TypedExprKind::ArrayLit(items),
    ) = (&expr.value, &kind)
        && let Some(first) = items
            .first()
            .and_then(|id| typed.exprs.ty.get(id.as_usize()))
        && items.iter().skip(1).any(|id| {
            typed
                .exprs
                .ty
                .get(id.as_usize())
                .is_some_and(|ty| ty != first)
        })
    {
        // type-sequence-elements-must-unify
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Sequence(SequenceCode::ElementsMustUnify)),
            "sequence literal elements must have one common type",
        ));
    }
    if let Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { count, .. }) = &expr.value
        && scope.expected_ty.is_none()
        && !matches!(count.value, Expr::Lit(Literal::Int(_)))
    {
        // type-sequence-repeat-requires-context
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Sequence(SequenceCode::RepeatRequiresContext)),
            "a runtime-count repeat requires an expected dynamic collection type",
        ));
    }
    if matches!(expr.value, Expr::Lit(ast::Literal::String(_)))
        && let Some(expected) = scope.expected_ty
        && !matches!(expected, Ty::Opaque(_))
        && expected != &resolved_ty
    {
        // type-text-literal-mismatch
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Text(TextCode::Mismatch)),
            "a text literal is a byte array; construct the desired value explicitly",
        ));
    }
    if let TypedExprKind::TakePtr(_) = &kind
        && resolved_ty.is_unresolved_address()
    {
        // type-raw-pointer-requires-expected-type
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Address(AddressCode::RawPointerRequiresContext)),
            "address-taking requires an expected raw-pointer type",
        ));
    }
    if matches!(kind, TypedExprKind::FixedArrayAlloc { .. })
        && matches!(resolved_ty, Ty::Opaque(name) if name.as_str() == "unresolved-array-construction")
    {
        // type-fixed-array-requires-expected-type
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Array(ArrayCode::FixedRequiresExpectedType)),
            "fixed-array construction requires an expected array type",
        ));
    }
    if let TypedExprKind::FixedArrayAlloc { size, .. } = &kind
        && let Ok(crate::representation::Repr::Array { length, .. }) =
            crate::representation::Repr::derive(&resolved_ty, Some(&typed.type_registry))
    {
        let matches_length = size.normalize_to_value().is_some_and(|value| {
                matches!(value, ast::ConstValue::Int(value) if ast::integer::to_u64(value) == Some(length))
            });
        if !matches_length {
            // type-fixed-array-length-mismatch
            flaws.push(flaw(
                &typed.span_table,
                expr.span_id,
                DiagnosticCode::Type(TypeCode::Array(ArrayCode::FixedLengthMismatch)),
                "fixed-array construction length does not match the selected array type",
            ));
        }
    }
    if let TypedExprKind::ArrayLit(items) = &kind
        && let Ok(crate::representation::Repr::Array { length, .. }) =
            crate::representation::Repr::derive(&resolved_ty, Some(&typed.type_registry))
        && length != items.len() as u64
    {
        // type-fixed-array-length-mismatch
        flaws.push(flaw(
            &typed.span_table,
            expr.span_id,
            DiagnosticCode::Type(TypeCode::Array(ArrayCode::FixedLengthMismatch)),
            "fixed-array literal length does not match the selected array type",
        ));
    }

    let expr_id = reserved_expr_id.unwrap_or(ExprId(typed.exprs.kind.len() as u32));

    if let TypedExprKind::InvalidOperator {
        op,
        lhs,
        rhs,
        error,
    } = &kind
    {
        let lhs_ty = &typed.exprs.ty[lhs.as_usize()];
        let rhs_ty = &typed.exprs.ty[rhs.as_usize()];
        let lhs_display = lhs_ty.format_for_hover();
        let rhs_display = rhs_ty.format_for_hover();
        let lhs_is_nominal = lhs_ty
            .named_instance_stripping_reference_wrappers()
            .is_some();
        let rhs_is_nominal = rhs_ty
            .named_instance_stripping_reference_wrappers()
            .is_some();
        let one_nominal = lhs_is_nominal || rhs_is_nominal;
        let is_comparison = matches!(
            op,
            ast::BinOp::Less
                | ast::BinOp::LessOrEqual
                | ast::BinOp::Greater
                | ast::BinOp::GreaterOrEqual
                | ast::BinOp::Equal
        );
        let incompatible_result_families = matches!(op, ast::BinOp::Equal)
            && matches!(lhs_ty, Ty::ResultFamily { .. })
            && matches!(rhs_ty, Ty::ResultFamily { .. })
            && lhs_ty != rhs_ty;
        let (code, message) = if incompatible_result_families {
            (
                // type-result-family-incompatible
                DiagnosticCode::Type(TypeCode::Result(ResultCode::FamilyIncompatible)),
                format!(
                    "comparison result families `{}` and `{}` have different semantic owners",
                    lhs_ty.format_for_hover(),
                    rhs_ty.format_for_hover()
                ),
            )
        } else if is_comparison {
            (
                DiagnosticCode::Type(TypeCode::Proposition(PropositionCode::NumericOperandsRequired)),
                "numeric comparisons require integer operands; use a named comparison function for custom orderings".to_string(),
            )
        } else {
            match error {
                crate::operator::OperatorResolutionError::NoVisibleContract => {
                    if one_nominal {
                        (
                            DiagnosticCode::Type(TypeCode::Operator(
                                // type-nominal-operator-unavailable
                                OperatorCode::NominalUnavailable,
                            )),
                            format!(
                                "operator `{}` is not provided for `{lhs_display}` and `{rhs_display}`",
                                op.symbol()
                            ),
                        )
                    } else {
                        (
                            DiagnosticCode::Type(TypeCode::Operator(
                                // type-no-visible-operator-contract
                                OperatorCode::NoVisibleContract,
                            )),
                            format!(
                                "no visible operator contract is registered for `{}`",
                                op.symbol()
                            ),
                        )
                    }
                }
                crate::operator::OperatorResolutionError::OperandDoesNotProvide => {
                    if one_nominal {
                        (
                            DiagnosticCode::Type(TypeCode::Operator(
                                // type-nominal-operator-unavailable
                                OperatorCode::NominalUnavailable,
                            )),
                            format!(
                                "operator `{}` is not provided for `{lhs_display}` and `{rhs_display}`",
                                op.symbol(),
                            ),
                        )
                    } else {
                        (
                            // type-operator-not-provided
                            DiagnosticCode::Type(TypeCode::Operator(OperatorCode::NotProvided)),
                            format!(
                                "operator `{}` is not provided for `{lhs_display}` and `{rhs_display}`",
                                op.symbol()
                            ),
                        )
                    }
                }
                crate::operator::OperatorResolutionError::Ambiguous => (
                    // type-ambiguous-operator
                    DiagnosticCode::Type(TypeCode::Operator(OperatorCode::Ambiguous)),
                    format!(
                        "multiple visible `{}` implementations apply to these operands",
                        op.symbol()
                    ),
                ),
            }
        };
        let mut diagnostic = Diagnostic::new(code, message)
            .with_arg("operator", op.symbol().to_string())
            .with_arg("left_type", lhs_display)
            .with_arg("right_type", rhs_display);
        if code == DiagnosticCode::Type(TypeCode::Operator(OperatorCode::NominalUnavailable)) {
            let nominal = if lhs_is_nominal { lhs_ty } else { rhs_ty };
            let help = if [lhs_ty, rhs_ty]
                .into_iter()
                .all(|ty| typed.type_registry.integer_validity_for_type(ty).is_some())
            {
                format!(
                    "for primitive integer arithmetic, project each named operand to its domain \
                     first, e.g. `raw is in {}: value`; use `raw` in the arithmetic expression",
                    nominal.without_reference_wrappers().format_for_hover(),
                )
            } else {
                format!(
                    "define or import an operator function annotated with `#infix('{}')` \
                     for these types",
                    op.symbol()
                )
            };
            diagnostic = diagnostic.with_help(help);
        }
        flaws.push(diagnostic);
    }
    if let TypedExprKind::IntrinsicCall { op, args, .. } = &kind {
        let operand_types: Vec<_> = args
            .iter()
            .map(|arg| typed.exprs.ty[arg.as_usize()].clone())
            .collect();
        anonymous_intrinsic_operand_call = operand_types.iter().all(|operand_type| {
            operand_type.is_int()
                && operand_type
                    .named_instance_stripping_reference_wrappers()
                    .is_none()
        });
        let anonymous_integer_result = resolved_ty.is_int()
            && resolved_ty
                .named_instance_stripping_reference_wrappers()
                .is_none();
        if anonymous_integer_result
            && anonymous_intrinsic_operand_call
            && args.len() == 2
            && let (Some(lhs), Some(rhs)) = (
                typed.exprs.facts.const_facts.const_value[args[0].as_usize()].as_ref(),
                typed.exprs.facts.const_facts.const_value[args[1].as_usize()].as_ref(),
            )
        {
            if let Some(result) = crate::anonymous_arithmetic::fold(
                *op,
                &[lhs.clone(), rhs.clone()],
                [&operand_types[0], &operand_types[1]],
            ) {
                match result {
                    Ok(value) => const_val = Some(value),
                    Err(diagnostic) => flaws.push(diagnostic),
                }
            }
        } else if !anonymous_integer_result && !anonymous_intrinsic_operand_call {
            if let Err(error) =
                op.validate_signature(&operand_types, &resolved_ty, Some(&typed.type_registry))
            {
                flaws.push(op.diagnostic(&error));
            } else {
                let values: Option<Vec<_>> = args
                    .iter()
                    .map(|arg| typed.exprs.facts.const_facts.const_value[arg.as_usize()].clone())
                    .collect();
                let source_width = operand_types
                    .first()
                    .and_then(|ty| typed.type_registry.integer_width_for_type(ty));
                let result_width = typed.type_registry.integer_width_for_type(&resolved_ty);
                if let (Some(values), Some(source_width), Some(result_width)) =
                    (values, source_width, result_width)
                {
                    match op.fold(&values, source_width, result_width) {
                        Ok(value) => const_val = value,
                        Err(error) => flaws.push(op.diagnostic(&error)),
                    }
                }
            }
        }
    }
    if let TypedExprKind::Binary { op, lhs, rhs } = &kind
        && *op == BinOp::Equal
        && let (Some(lhs), Some(rhs)) = (
            typed.exprs.facts.const_facts.const_value[lhs.as_usize()].as_ref(),
            typed.exprs.facts.const_facts.const_value[rhs.as_usize()].as_ref(),
        )
    {
        const_val = lhs.eval_binop(op, rhs);
    }
    if matches!(kind, TypedExprKind::IntrinsicCall { .. })
        && ((resolved_ty.is_int()
            && resolved_ty
                .named_instance_stripping_reference_wrappers()
                .is_none())
            || anonymous_intrinsic_operand_call)
        && !scope
            .current_def_id
            .and_then(|definition| typed.defs.get(&definition))
            .is_some_and(|definition| definition.is_constant)
    {
        const_val = None;
    }
    if let TypedExprKind::Reassign { name, .. } = &kind {
        if env.constants.contains(name) {
            flaws.push(
                // type-reassign-constant
                Diagnostic::new(
                    BindingCode::ConstantReassignment,
                    format!("cannot reassign constant `{}`", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            );
        }
        if env
            .places
            .get(name)
            .and_then(|place| typed.places.get(place.0 as usize))
            .is_some_and(|place| !place.mutable)
        {
            flaws.push(
                // type-rebind-immutable
                Diagnostic::new(
                    BindingCode::RebindImmutable,
                    format!("cannot rebind immutable value `{}`", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            );
        }
    }
    let contextual_result_ty = match &kind {
        TypedExprKind::TagCall { variant_id, .. } => scope
            .current_def_id
            .and_then(|def_id| typed.defs.get(&def_id))
            .map(|bind| &bind.return_type)
            .filter(|ty| matches!(
                ty,
                Ty::ResultFamily { alternatives, .. }
                    if alternatives.iter().any(|alternative| alternative.label == variant_id.name)
            )),
        _ => None,
    };
    check_type_flaws(
        expr_id,
        &kind,
        contextual_result_ty.unwrap_or(&resolved_ty),
        FlawContext {
            typed,
            name_span_id,
            locals: scope.locals,
            local_var_types: &env.types,
            flaws: &mut flaws,
            value_required: scope.value_required,
        },
    );

    // If the expression has a substituted type (from const-generic param resolution),
    // use it instead of the inferred type.
    let mut final_ty = match &kind {
        TypedExprKind::Ref(inner) if matches!(expr.value, Expr::Ref { .. }) => {
            let Expr::Ref { mutable, .. } = &expr.value else {
                unreachable!()
            };
            Ty::Ref {
                inner: Box::new(typed.exprs.ty[inner.as_usize()].clone()),
                mutable: *mutable,
            }
        }
        TypedExprKind::ResultRow(ids) => {
            let names = match &expr.value {
                Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elems)) => {
                    elems.iter().map(|elem| elem.name).collect::<Vec<_>>()
                }
                _ => vec![None; ids.len()],
            };
            Ty::Sequence(
                ids.iter()
                    .enumerate()
                    .map(|(index, id)| ast::ty::SequenceTypeElement {
                        name: names[index].or_else(|| match scope.expected_ty {
                            Some(Ty::Sequence(fields)) => {
                                fields.get(index).and_then(|field| field.name)
                            }
                            _ => None,
                        }),
                        ty: Box::new(typed.exprs.ty[id.as_usize()].clone()),
                    })
                    .collect(),
            )
        }
        TypedExprKind::TagCall { .. } if contextual_result_ty.is_some() => {
            contextual_result_ty.cloned().unwrap()
        }
        TypedExprKind::FnCall {
            substituted_ty: Some(ty),
            ..
        } => ty.clone(),
        TypedExprKind::Bind { name, .. } | TypedExprKind::Reassign { name, .. } => env
            .types
            .get(name)
            .cloned()
            .unwrap_or_else(|| resolved_ty.clone()),
        TypedExprKind::When(_) if !scope.value_required => Ty::Unit,
        TypedExprKind::When(when_expr) => scope
            .expected_ty
            .cloned()
            .or_else(|| {
                let domains = when_expr
                    .arms
                    .iter()
                    .filter_map(|arm| {
                        let body = match arm {
                            TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. } => {
                                *body
                            }
                            TypedWhenArm::Else(body, _) => *body,
                        };
                        if crate::analysis::expression_returns(typed.resolved_program(), body) {
                            return None;
                        }
                        Some(
                            typed.exprs.ty[body.as_usize()]
                                .anonymous_integer_validity()
                                .map(|validity| validity.domain().clone()),
                        )
                    })
                    .collect::<Option<Vec<_>>>()?;
                (!domains.is_empty()).then(|| Ty::AnonymousInteger {
                    validity: ast::integer::IntegerValidity::new(
                        ast::integer::IntegerDomain::union(domains),
                    ),
                })
            })
            .or_else(|| {
                when_expr.arms.iter().find_map(|arm| {
                    let body = match arm {
                        TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. } => *body,
                        TypedWhenArm::Else(body, _) => *body,
                    };
                    if crate::analysis::expression_returns(typed.resolved_program(), body) {
                        return None;
                    }
                    typed
                        .exprs
                        .ty
                        .get(body.as_usize())
                        .filter(|ty| !matches!(ty, Ty::UnresolvedLiteral(_)))
                        .cloned()
                })
            })
            .unwrap_or_else(|| resolved_ty.clone()),
        TypedExprKind::Block { value, .. } => value
            .and_then(|value| typed.exprs.ty_of(value).cloned())
            .unwrap_or(Ty::Unit),
        TypedExprKind::Return { .. } | TypedExprKind::If(_) => Ty::Unit,
        _ => resolved_ty.clone(),
    };
    if matches!(
        final_ty,
        Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
    ) && let Some(ast::ConstValue::Int(value)) = &const_val
    {
        let is_compile_time_definition = scope
            .current_def_id
            .and_then(|def_id| typed.defs.get(&def_id))
            .is_some_and(|bind| bind.is_constant);
        let selected = scope
            .expected_ty
            .and_then(|expected| materialized_integer_expected(expected, &typed.type_registry));
        if let Some(expected) = selected {
            match crate::integer_literal::validate_with_registry(
                *value,
                &expected,
                &typed.type_registry,
            ) {
                Ok(()) => final_ty = expected,
                Err(_) if is_compile_time_definition && scope.expected_ty.is_none() => {
                    let width = integer_width_for_exact_value(*value);
                    final_ty = Ty::anonymous_integer_for_width(width, *value < i256::I256::from(0));
                }
                Err(error) => flaws.push(
                    crate::integer_literal::diagnostic(*value, &expected, &error)
                        .at_span_id(expr.span_id, &typed.span_table),
                ),
            }
        } else if scope.allow_literal_materialization
            && scope.expected_ty.is_none_or(|ty| {
                matches!(ty, Ty::UnresolvedLiteral(_))
                    || is_generic_opaque_ty(ty, &typed.type_registry)
            })
        {
            let width = integer_width_for_exact_value(*value);
            let signed = *value < i256::I256::from(0);
            final_ty = Ty::anonymous_integer_for_width(width, signed);
        }
    }
    if matches!(
        kind,
        TypedExprKind::IntrinsicCall {
            op: IntrinsicOp::BitsAdd | IntrinsicOp::BitsSubtract | IntrinsicOp::BitsMultiply,
            ..
        }
    ) && matches!(final_ty, Ty::AnonymousInteger { .. })
        && let Some(ast::ConstValue::Int(value)) = &const_val
    {
        final_ty = Ty::anonymous_integer_for_width(
            integer_width_for_exact_value(*value),
            *value < i256::I256::from(0),
        );
    }
    if let Some(expected) = scope.expected_ty
        && super::return_types::same_union_layout(&typed.type_registry, &final_ty, expected)
    {
        final_ty = expected.clone();
    }
    let injection = scope.expected_ty.and_then(|expected| {
        let Ty::Union { name, variants, .. } =
            typed.type_registry.resolved_definition_for_type(expected)
        else {
            return None;
        };
        let mut candidates = variants.iter().enumerate().filter(|(_, variant)| {
            variant.kind == ast::ty::UnionVariantKind::Type
                && variant
                    .fields
                    .first()
                    .is_some_and(|(_, ty)| **ty == final_ty)
        });
        let (index, variant) = candidates.next()?;
        candidates
            .next()
            .is_none()
            .then_some((name, index, variant.name, expected.clone()))
    });
    if injection.is_none()
        && let (
            Some(Ty::Named {
                instance: expected, ..
            }),
            Ty::Named {
                instance: actual, ..
            },
        ) = (scope.expected_ty, &final_ty)
    {
        if actual.declaration != expected.declaration
            || (!named_instance_has_symbolic_argument(actual)
                && actual.arguments != expected.arguments)
        {
            // type-return-mismatch
            flaws.push(flaw(
                &typed.span_table,
                expr.span_id,
                DiagnosticCode::Type(TypeCode::Result(ResultCode::ReturnMismatch)),
                "value's nominal type does not match the expected type",
            ));
        }
    }
    if let Some(expected) = scope.expected_ty
        && !matches!(kind, TypedExprKind::If(_) | TypedExprKind::Loop(_))
        && !is_generic_opaque_ty(expected, &typed.type_registry)
        && final_ty != *expected
        && injection.is_none()
        && !crate::analysis::kind_returns(typed.resolved_program(), &kind, |child| {
            crate::analysis::expression_returns(typed.resolved_program(), *child)
        })
        && (matches!(
            typed.type_registry.resolved_definition_for_type(&final_ty),
            Ty::Union { .. }
        ) || matches!(
            typed.type_registry.resolved_definition_for_type(expected),
            Ty::Union { .. }
        ))
    {
        flaws.push(
            // type-incompatible-union-value
            Diagnostic::new(
                UnionCode::ValueIncompatible,
                format!(
                    "value of type `{}` does not match expected `{}` or its proven member",
                    final_ty.format_for_hover(),
                    expected.format_for_hover(),
                ),
            )
            .at_span_id(expr.span_id, &typed.span_table),
        );
    }
    let target_group = target_group_for_kind(&kind, typed, scope, env, &mut flaws);
    let result_target_groups = result_target_groups_for_kind(&kind, typed, scope, env);
    let place = place_for_kind(&kind, &final_ty, typed, env);
    let place_version = place_version_for_kind(&kind, place, typed, env);
    if let Some(version) = place_version
        && let Some(value) = match &kind {
            TypedExprKind::Bind { body, .. } => Some(*body),
            TypedExprKind::Reassign { value, .. } => Some(*value),
            _ => None,
        }
        && let Some(domain) = typed
            .exprs
            .integer_knowledge_of(value)
            .and_then(Option::as_ref)
            .and_then(super::place::knowledge_domain)
    {
        env.integer_domains.insert(version, domain);
    }
    let integer_knowledge = place_version
        .and_then(|version| env.integer_domains.get(&version).cloned())
        .map(ast::integer::IntegerKnowledge::Domain)
        .or_else(|| aggregate_projection_integer_knowledge(&kind, typed, place_version))
        .or_else(|| masked_integer_knowledge(&kind, typed))
        .or_else(|| {
            if let TypedExprKind::Cast { expr, ty } = &kind
                && crate::discriminant::is_numeric_cast(
                    &typed.type_registry,
                    &typed.exprs.ty[expr.index()],
                    ty,
                )
                && typed.type_registry.integer_validity_for_type(ty).is_some()
            {
                crate::discriminant::cast_source_domain(typed, *expr, ty)
                    .map(ast::integer::IntegerKnowledge::Domain)
            } else {
                None
            }
        })
        .or_else(|| integer_knowledge_for(&final_ty, const_val.as_ref(), &typed.type_registry));
    let result_evidence = match &kind {
        TypedExprKind::Bind { body, .. }
        | TypedExprKind::Block {
            value: Some(body), ..
        } => typed.exprs.value_evidence_of(*body).cloned(),
        TypedExprKind::FnCall {
            target, args: None, ..
        } if env.locals.contains(&target.0) => env.result_evidence.get(&target.0).cloned(),
        TypedExprKind::AggregateGet { base, index } => typed
            .exprs
            .projected_result_evidence_of(*base)
            .and_then(|evidence| evidence.get(&vec![*index]).cloned()),
        _ => value_evidence_for(&kind, &final_ty, typed),
    }
    .map(Box::new);
    let projected_result_evidence = projected_value_evidence_for(&kind, typed, env);
    let sequence_literal = typed_sequence_literal(&expr.value, &kind).map(Box::new);
    let literal_materialization =
        sequence_literal_materialization_plan(&expr.value, &kind, &final_ty, &typed.type_registry)
            .map(Box::new);
    if reserved_expr_id.is_some() {
        let index = expr_id.as_usize();
        typed.exprs.kind[index] = kind;
        typed.exprs.ty[index] = final_ty;
        typed.exprs.span[index] = expr.span_id;
        typed.exprs.facts.const_facts.const_value[index] = const_val;
        typed.exprs.facts.const_facts.const_object[index] = None;
        typed.exprs.facts.const_facts.sequence_literal[index] = sequence_literal;
        typed.exprs.facts.const_facts.literal_materialization[index] = literal_materialization;
        typed.exprs.facts.semantic_facts.integer_knowledge[index] = integer_knowledge;
        typed.exprs.facts.semantic_facts.result_evidence[index] = result_evidence;
        typed.exprs.facts.semantic_facts.projected_result_evidence[index] =
            projected_result_evidence;
        typed.exprs.facts.semantic_facts.availability[index] = Availability::Unknown;
        typed.exprs.set_ownership_facts(
            index,
            target_group,
            result_target_groups,
            place,
            place_version,
        );
        typed.exprs.facts.semantic_facts.flaws[index].extend(flaws);
    } else {
        typed.exprs.kind.push(kind);
        typed.exprs.ty.push(final_ty);
        typed.exprs.span.push(expr.span_id);
        typed.exprs.facts.const_facts.const_value.push(const_val);
        typed.exprs.facts.const_facts.const_object.push(None);
        typed
            .exprs
            .facts
            .const_facts
            .sequence_literal
            .push(sequence_literal);
        typed
            .exprs
            .facts
            .const_facts
            .literal_materialization
            .push(literal_materialization);
        typed
            .exprs
            .facts
            .semantic_facts
            .integer_knowledge
            .push(integer_knowledge);
        typed
            .exprs
            .facts
            .semantic_facts
            .result_evidence
            .push(result_evidence);
        typed
            .exprs
            .facts
            .semantic_facts
            .projected_result_evidence
            .push(projected_result_evidence);
        typed
            .exprs
            .facts
            .semantic_facts
            .availability
            .push(Availability::Unknown);
        typed
            .exprs
            .push_ownership_facts(target_group, result_target_groups, place, place_version);
        typed.exprs.facts.semantic_facts.flaws.push(flaws);
    }

    if let Some((union, discriminant, name, ty)) = injection {
        return append_derived_expr(
            typed,
            expr_id,
            TypedExprKind::TagCall {
                variant_id: crate::typed::VariantId {
                    union: TagId(union),
                    name,
                },
                discriminant,
                args: Some(vec![expr_id]),
                field_names: vec![Intern::from_ref("value")],
            },
            ty,
        );
    }
    expr_id
}

pub(super) fn append_derived_expr(
    typed: &mut TypedFileAst,
    source: ExprId,
    kind: TypedExprKind,
    ty: Ty,
) -> ExprId {
    let projection = matches!(kind, TypedExprKind::UnionPayload { .. });
    let target_group = match &kind {
        TypedExprKind::AggregateGet { index, .. } => {
            typed.exprs.facts.ownership_facts.result_target_groups[source.as_usize()]
                .get(*index)
                .cloned()
                .flatten()
        }
        _ => typed.exprs.target_group_of(source).cloned().flatten(),
    };
    let id = ExprId(typed.exprs.kind.len() as u32);
    let span = typed.exprs.span[source.as_usize()];
    let place = projection
        .then(|| typed.exprs.place_of(source).flatten())
        .flatten();
    let place_version = projection
        .then(|| typed.exprs.place_version_of(source).flatten())
        .flatten();
    typed.exprs.push(crate::typed::TypedExpr {
        kind,
        ty: ty.clone(),
        span,
        facts: crate::typed::ExprFacts {
            const_facts: crate::typed::ConstFacts {
                const_value: None,
                const_object: None,
                sequence_literal: None,
                literal_materialization: None,
            },
            semantic_facts: crate::typed::SemanticFacts {
                integer_knowledge: integer_knowledge_for(&ty, None, &typed.type_registry),
                result_evidence: None,
                projected_result_evidence: Default::default(),
                availability: Availability::Unknown,
                flaws: Vec::new(),
            },
            ownership_facts: crate::typed::OwnershipFacts {
                references: Default::default(),
                reference_reads: Default::default(),
                target_group,
                result_target_groups: Vec::new(),
                place,
                place_version,
            },
        },
    });
    id
}

fn typed_sequence_literal(
    source: &Expr,
    kind: &TypedExprKind,
) -> Option<crate::typed::TypedSequenceLiteral> {
    match (source, kind) {
        (
            Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elems)),
            TypedExprKind::ResultRow(items),
        ) if elems.is_empty() => Some(crate::typed::TypedSequenceLiteral::Elements(items.clone())),
        (
            Expr::SequenceLiteral(ast::SequenceLiteral::Elements(_)),
            TypedExprKind::ArrayLit(items),
        ) => Some(crate::typed::TypedSequenceLiteral::Elements(items.clone())),
        (
            Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { .. }),
            TypedExprKind::FixedArrayAlloc { init, size },
        ) => Some(crate::typed::TypedSequenceLiteral::Repeat {
            seed: *init,
            count: size.clone(),
        }),
        _ => None,
    }
}

fn sequence_literal_materialization_plan(
    source: &Expr,
    kind: &TypedExprKind,
    ty: &Ty,
    registry: &crate::TypeRegistry,
) -> Option<crate::typed::LiteralMaterializationPlan> {
    if !matches!(source, Expr::SequenceLiteral(_)) {
        return None;
    }
    let length = match kind {
        TypedExprKind::ArrayLit(items) => ast::NormalExpr::from(items.len() as i128),
        TypedExprKind::FixedArrayAlloc { size, .. } => size.clone(),
        _ => return None,
    };
    sequence_literal_materialization_plan_for_length(length, ty, registry)
}

pub(super) fn sequence_element_argument<'a>(
    instance: &'a ast::ty::NamedTypeInstance,
    former: &crate::type_registry::SequenceLiteralAcceptance,
) -> Option<(usize, &'a ast::TyArg)> {
    instance
        .arguments
        .iter()
        .enumerate()
        .find_map(|(index, (parameter, argument))| {
            (*parameter == former.element_parameter).then_some((index, argument))
        })
}

fn sequence_builder_member(display_name: &str, member: Intern<String>) -> DefId {
    DefId::named(Intern::new(format!("{display_name}.{}", member.as_str())))
}

fn sequence_literal_materialization_plan_for_length(
    length: ast::NormalExpr,
    ty: &Ty,
    registry: &crate::TypeRegistry,
) -> Option<crate::typed::LiteralMaterializationPlan> {
    match Ty::without_reference_wrappers(ty) {
        Ty::Array { elem, .. } => Some(crate::typed::LiteralMaterializationPlan::FixedArray {
            declaration: None,
            element: elem.as_ref().clone(),
            length,
        }),
        Ty::Named { instance, .. } => {
            let declaration = registry.declaration_for_instance(instance)?;
            let former = declaration.sequence_literal.as_ref()?;
            let (_, element) = sequence_element_argument(instance, former)?;
            let ast::TyArg::Type(element) = element else {
                return None;
            };
            match former.materialization {
                crate::SequenceLiteralMaterialization::FixedArray { .. } => {
                    Some(crate::typed::LiteralMaterializationPlan::FixedArray {
                        declaration: Some(instance.declaration),
                        element: element.as_ref().clone(),
                        length,
                    })
                }
                crate::SequenceLiteralMaterialization::OwnedBuilder {
                    empty_member,
                    push_member,
                } => Some(crate::typed::LiteralMaterializationPlan::OwnedSequence {
                    declaration: instance.declaration,
                    element: element.as_ref().clone(),
                    empty: sequence_builder_member(declaration.display_name.as_str(), empty_member),
                    push: sequence_builder_member(declaration.display_name.as_str(), push_member),
                }),
            }
        }
        _ => None,
    }
}

pub(crate) fn contextualize_local_literal(
    typed: &mut TypedFileAst,
    expression: ExprId,
    expected: &Ty,
    env: &mut FlowState,
) {
    let Some(version) = typed.exprs.place_version_of(expression).flatten() else {
        return;
    };
    let Some(crate::typed::TypedPlaceVersion {
        place,
        origin: crate::typed::PlaceVersionOrigin::Initializer(initializer),
        ..
    }) = typed.place_versions.get(version.0 as usize)
    else {
        return;
    };
    let place = *place;
    let initializer = *initializer;
    if typed.places[place.0 as usize].explicit_contract {
        return;
    }
    if !contextualize_literal_value(typed, initializer, expected) {
        return;
    }

    typed.exprs.ty[expression.as_usize()] = expected.clone();
    typed.places[place.0 as usize].ty = expected.clone();
    typed.place_versions[version.0 as usize].ty = expected.clone();
    let name = typed.places[place.0 as usize].name;
    env.types.insert(name, expected.clone());

    for index in 0..typed.exprs.kind.len() {
        if matches!(
            typed.exprs.kind[index],
            TypedExprKind::Bind { body, .. } if body == initializer
        ) {
            typed.exprs.ty[index] = expected.clone();
        }
    }
}

pub(super) fn contextualize_literal_value(
    typed: &mut TypedFileAst,
    expression: ExprId,
    expected: &Ty,
) -> bool {
    if let Some(ConstValue::Int(value)) =
        typed.exprs.facts.const_facts.const_value[expression.as_usize()].as_ref()
        && (typed.exprs.ty[expression.as_usize()]
            .named_instance()
            .is_none()
            || typed.exprs.ty[expression.as_usize()] == *expected)
        && typed
            .type_registry
            .integer_validity_for_type(expected)
            .is_some()
    {
        let value = *value;
        if let Err(error) =
            crate::integer_literal::validate_with_registry(value, expected, &typed.type_registry)
        {
            let span = typed.exprs.span[expression.as_usize()];
            typed.exprs.facts.semantic_facts.flaws[expression.as_usize()].push(
                crate::integer_literal::diagnostic(value, expected, &error)
                    .at_span_id(span, &typed.span_table),
            );
            return false;
        }
        typed.exprs.ty[expression.as_usize()] = expected.clone();
        typed.exprs.facts.semantic_facts.integer_knowledge[expression.as_usize()] =
            integer_knowledge_for(
                expected,
                Some(&ConstValue::Int(value)),
                &typed.type_registry,
            );
        return true;
    }

    let literal = typed.exprs.facts.const_facts.sequence_literal[expression.as_usize()].clone();
    let Some(literal) = literal else {
        return false;
    };
    let length = match literal.as_ref() {
        crate::typed::TypedSequenceLiteral::Elements(items) => {
            ast::NormalExpr::from(items.len() as i128)
        }
        crate::typed::TypedSequenceLiteral::Repeat { count, .. } => count.clone(),
    };
    let Some(plan) =
        sequence_literal_materialization_plan_for_length(length, expected, &typed.type_registry)
    else {
        return false;
    };
    let element = match &plan {
        crate::typed::LiteralMaterializationPlan::FixedArray { element, .. }
        | crate::typed::LiteralMaterializationPlan::OwnedSequence { element, .. } => {
            element.clone()
        }
    };
    let elements = match *literal {
        crate::typed::TypedSequenceLiteral::Elements(items) => items,
        crate::typed::TypedSequenceLiteral::Repeat { seed, .. } => vec![seed],
    };
    if !elements
        .into_iter()
        .all(|item| contextualize_literal_value(typed, item, &element))
    {
        return false;
    }
    if let TypedExprKind::ResultRow(items) = &typed.exprs.kind[expression.as_usize()]
        && items.is_empty()
    {
        typed.exprs.kind[expression.as_usize()] = TypedExprKind::ArrayLit(Vec::new());
    }
    typed.exprs.facts.const_facts.literal_materialization[expression.as_usize()] =
        Some(Box::new(plan));
    contextualized_literal_type(typed, expression, expected);
    true
}

fn contextualized_literal_type(typed: &mut TypedFileAst, expression: ExprId, expected: &Ty) {
    typed.exprs.ty[expression.as_usize()] = expected.clone();
    typed.exprs.facts.semantic_facts.flaws[expression.as_usize()].retain(|flaw| {
        !matches!(
            flaw.code,
            DiagnosticCode::Type(TypeCode::Sequence(SequenceCode::LiteralRequiresContext))
                | DiagnosticCode::Type(TypeCode::Text(TextCode::RequiresContext))
        )
    });
}

fn retarget_literal_materialization(
    plan: &mut Option<Box<crate::typed::LiteralMaterializationPlan>>,
    ty: &Ty,
    registry: &crate::TypeRegistry,
) {
    if plan.is_none() {
        return;
    }
    let Ty::Named { instance, .. } = Ty::without_reference_wrappers(ty) else {
        return;
    };
    let Some(semantics) = registry.declaration_for_instance(instance) else {
        return;
    };
    let Some(former) = semantics.sequence_literal.as_ref() else {
        return;
    };
    let Some((_, ast::TyArg::Type(contextual_element))) =
        sequence_element_argument(instance, former)
    else {
        return;
    };
    match former.materialization {
        crate::SequenceLiteralMaterialization::FixedArray { .. } => {
            if let Some(crate::typed::LiteralMaterializationPlan::FixedArray {
                declaration,
                element,
                ..
            }) = plan.as_deref_mut()
            {
                *declaration = Some(instance.declaration);
                *element = contextual_element.as_ref().clone();
            }
        }
        crate::SequenceLiteralMaterialization::OwnedBuilder {
            empty_member,
            push_member,
        } => {
            *plan = Some(Box::new(
                crate::typed::LiteralMaterializationPlan::OwnedSequence {
                    declaration: instance.declaration,
                    element: contextual_element.as_ref().clone(),
                    empty: sequence_builder_member(semantics.display_name.as_str(), empty_member),
                    push: sequence_builder_member(semantics.display_name.as_str(), push_member),
                },
            ));
        }
    }
}

pub(crate) fn materialize_owned_sequence_literals(typed: &mut TypedFileAst) {
    let expression_count = typed.exprs.kind.len();
    for index in 0..expression_count {
        let Some(plan) = typed.exprs.facts.const_facts.literal_materialization[index].clone()
        else {
            continue;
        };
        let crate::typed::LiteralMaterializationPlan::OwnedSequence { empty, push, .. } = *plan
        else {
            continue;
        };
        let Some(literal) = typed.exprs.facts.const_facts.sequence_literal[index].clone() else {
            continue;
        };
        let items = match *literal {
            crate::typed::TypedSequenceLiteral::Elements(items) => items,
            crate::typed::TypedSequenceLiteral::Repeat { seed, count } => {
                let Some(count) = count
                    .as_const_int()
                    .and_then(|value| usize::try_from(value).ok())
                else {
                    let span = typed.exprs.span[index];
                    // type-sequence-builder-repeat-requires-constant-count
                    typed.exprs.facts.semantic_facts.flaws[index].push(flaw(
                        &typed.span_table,
                        span,
                        DiagnosticCode::Type(TypeCode::Sequence(
                            SequenceCode::BuilderRepeatRequiresConstantCount,
                        )),
                        "owned sequence builders require a compile-time repeat count",
                    ));
                    continue;
                };
                if count == 0 {
                    Vec::new()
                } else if count == 1 {
                    vec![seed]
                } else {
                    let Some(constant) =
                        typed.exprs.facts.const_facts.const_value[seed.as_usize()].clone()
                    else {
                        let span = typed.exprs.span[index];
                        // type-sequence-repeat-seed-not-copyable
                        typed.exprs.facts.semantic_facts.flaws[index].push(flaw(
                            &typed.span_table,
                            span,
                            DiagnosticCode::Type(TypeCode::Sequence(
                                SequenceCode::RepeatSeedNotCopyable,
                            )),
                            "repeated owned sequence seed must be compile-time rematerializable",
                        ));
                        continue;
                    };
                    let mut items = Vec::with_capacity(count);
                    items.push(seed);
                    for _ in 1..count {
                        items.push(append_sequence_builder_rematerialize(
                            typed,
                            seed,
                            constant.clone(),
                        ));
                    }
                    items
                }
            }
        };
        let ty = typed.exprs.ty[index].clone();
        let span = typed.exprs.span[index];
        let mut current = append_sequence_builder_call(typed, empty, Vec::new(), ty.clone(), span);
        if let Some((last, prefix)) = items.split_last() {
            for item in prefix {
                current = append_sequence_builder_call(
                    typed,
                    push,
                    vec![current, *item],
                    ty.clone(),
                    span,
                );
            }
            typed.exprs.kind[index] = TypedExprKind::FnCall {
                target: push,
                args: Some(vec![current, *last]),
                substituted_ty: Some(ty),
            };
        } else {
            typed.exprs.kind[index] = TypedExprKind::FnCall {
                target: empty,
                args: Some(Vec::new()),
                substituted_ty: Some(ty),
            };
        }
        typed.exprs.facts.const_facts.const_value[index] = None;
        typed.exprs.facts.const_facts.const_object[index] = None;
    }
}

fn append_sequence_builder_call(
    typed: &mut TypedFileAst,
    target: DefId,
    args: Vec<ExprId>,
    ty: Ty,
    span: SpanId,
) -> ExprId {
    typed.exprs.push_placeholder(
        TypedExprKind::FnCall {
            target,
            args: Some(args),
            substituted_ty: Some(ty.clone()),
        },
        ty,
        span,
    )
}

fn append_sequence_builder_rematerialize(
    typed: &mut TypedFileAst,
    source: ExprId,
    constant: ast::ConstValue,
) -> ExprId {
    let source_ty = typed.exprs.ty[source.as_usize()].clone();
    let source_span = typed.exprs.span[source.as_usize()];
    let source_integer_knowledge =
        typed.exprs.facts.semantic_facts.integer_knowledge[source.as_usize()].clone();
    let source_result_evidence =
        typed.exprs.facts.semantic_facts.result_evidence[source.as_usize()].clone();
    let id = typed.exprs.push_placeholder(
        TypedExprKind::Rematerialize {
            source,
            constant: constant.clone(),
        },
        source_ty,
        source_span,
    );
    let index = id.as_usize();
    typed.exprs.facts.const_facts.const_value[index] = Some(constant);
    typed.exprs.facts.semantic_facts.integer_knowledge[index] = source_integer_knowledge;
    typed.exprs.facts.semantic_facts.result_evidence[index] = source_result_evidence;
    typed.exprs.facts.semantic_facts.availability[index] = Availability::CompileTime;
    id
}

fn place_version_for_kind(
    kind: &TypedExprKind,
    place: Option<crate::typed::PlaceId>,
    typed: &mut TypedFileAst,
    env: &mut FlowState,
) -> Option<crate::typed::PlaceVersionId> {
    let projected_write = match kind {
        TypedExprKind::RecordSet { value, .. }
        | TypedExprKind::AggregateSet { value, .. }
        | TypedExprKind::BufSet { value, .. } => Some(*value),
        TypedExprKind::Reassign { value, .. }
            if place.is_some_and(|place| typed.places[place.0 as usize].parent.is_some()) =>
        {
            Some(*value)
        }
        _ => None,
    };
    if let (Some(place), Some(value)) = (place, projected_write) {
        let predecessor = current_version_for_place(typed, env, place);
        let version = typed.add_place_version(
            place,
            crate::typed::PlaceVersionOrigin::Rebind { value, predecessor },
            typed.places[place.0 as usize].ty.clone(),
            typed.exprs.integer_knowledge_of(value).cloned().flatten(),
        );
        env.set_projected_version(place, version);
        let mut parent = typed.places[place.0 as usize].parent;
        while let Some(ancestor) = parent {
            let info = &typed.places[ancestor.0 as usize];
            let name = info.name;
            parent = info.parent;
            let mutation = typed.add_place_version(
                ancestor,
                crate::typed::PlaceVersionOrigin::Mutation(value),
                info.ty.clone(),
                None,
            );
            if parent.is_some() {
                env.set_projected_version(ancestor, mutation);
            } else {
                env.set_place_version(name, mutation);
            }
        }
        return Some(version);
    }
    match kind {
        TypedExprKind::FnCall {
            target, args: None, ..
        } => env.place_versions.get(&target.0).copied(),
        TypedExprKind::Block {
            value: Some(value), ..
        } => typed.exprs.place_version_of(*value).flatten(),
        TypedExprKind::SelfRef { .. } => env.place_versions.get(&Intern::from_ref("self")).copied(),
        TypedExprKind::Bind { name, .. } | TypedExprKind::Reassign { name, .. } => {
            env.place_versions.get(name).copied()
        }
        TypedExprKind::AggregateGet { .. }
        | TypedExprKind::BufGet { .. }
        | TypedExprKind::Deref(_) => {
            place.and_then(|place| current_version_for_place(typed, env, place))
        }
        _ => None,
    }
}

fn place_for_kind(
    kind: &TypedExprKind,
    ty: &Ty,
    typed: &mut TypedFileAst,
    env: &mut FlowState,
) -> Option<crate::typed::PlaceId> {
    match kind {
        TypedExprKind::FnCall {
            target, args: None, ..
        } => env.places.get(&target.0).copied(),
        TypedExprKind::Block {
            value: Some(value), ..
        } => typed.exprs.place_of(*value).flatten(),
        TypedExprKind::SelfRef { .. } => env.places.get(&Intern::from_ref("self")).copied(),
        TypedExprKind::Bind { name, .. } => env.places.get(name).copied(),
        TypedExprKind::Reassign { name, .. } => {
            let parent = env.places.get(name).copied()?;
            if let Some(referent) = typed.borrowed_parameter_referent(parent).cloned() {
                let base_version = env.place_versions.get(name).copied()?;
                Some(projected_place_from_parent(
                    typed,
                    env,
                    parent,
                    base_version,
                    crate::typed::PlaceProjection::Deref,
                    referent,
                ))
            } else {
                Some(parent)
            }
        }
        TypedExprKind::AggregateGet { base, index }
        | TypedExprKind::AggregateSet { base, index, .. } => {
            let base_ty = typed.exprs.ty_of(*base)?;
            let base_ty = reference_pointee_for_type(base_ty, Some(&typed.type_registry))
                .unwrap_or_else(|| base_ty.clone());
            let base_definition = typed.type_registry.resolved_definition_for_type(&base_ty);
            let projection_ty = match base_definition {
                Ty::Record { fields, .. } => fields
                    .get(*index)
                    .map(|(_, field)| (**field).clone())
                    .unwrap_or_else(|| ty.clone()),
                Ty::Sequence(fields) => fields
                    .get(*index)
                    .map(|field| (*field.ty).clone())
                    .unwrap_or_else(|| ty.clone()),
                Ty::Array { elem, .. } => elem.as_ref().clone(),
                _ => ty.clone(),
            };
            let projection = if matches!(
                typed.type_registry.resolved_definition_for_type(&base_ty),
                Ty::Array { .. }
            ) {
                crate::typed::PlaceProjection::Item(crate::typed::TargetIndex::Symbolic(
                    ast::NormalExpr::from(*index as i128),
                ))
            } else {
                crate::typed::PlaceProjection::Field(*index)
            };
            projected_place(typed, env, *base, projection, projection_ty)
        }
        TypedExprKind::RecordSet { base, field, .. } => {
            let base_ty = typed.exprs.ty_of(*base)?;
            let base_ty = reference_pointee_for_type(base_ty, Some(&typed.type_registry))
                .unwrap_or_else(|| base_ty.clone());
            let base_definition = typed.type_registry.resolved_definition_for_type(&base_ty);
            let (index, projection_ty) = match base_definition {
                Ty::Record { fields, .. } => {
                    let index = fields.iter().position(|(name, _)| name == field)?;
                    (index, fields[index].1.as_ref().clone())
                }
                _ => return None,
            };
            projected_place(
                typed,
                env,
                *base,
                crate::typed::PlaceProjection::Field(index),
                projection_ty,
            )
        }
        TypedExprKind::SliceGet { buf, .. } => projected_place(
            typed,
            env,
            *buf,
            crate::typed::PlaceProjection::Item(crate::typed::TargetIndex::Unknown),
            ty.clone(),
        ),
        TypedExprKind::BufGet { buf, index } | TypedExprKind::BufSet { buf, index, .. } => {
            let projection_ty = super::lower_ty::extract_element_type(
                typed.exprs.ty_of(*buf)?,
                Some(&typed.type_registry),
            );
            projected_place(
                typed,
                env,
                *buf,
                crate::typed::PlaceProjection::Item(target_index_for_expr(typed, *index)),
                projection_ty,
            )
        }
        TypedExprKind::Deref(base) => {
            let projection_ty = typed
                .exprs
                .ty_of(*base)
                .and_then(Ty::pointee_ty)
                .cloned()
                .unwrap_or_else(|| ty.clone());
            projected_place(
                typed,
                env,
                *base,
                crate::typed::PlaceProjection::Deref,
                projection_ty,
            )
        }
        _ => None,
    }
}

fn projected_place(
    typed: &mut TypedFileAst,
    env: &mut FlowState,
    base: ExprId,
    projection: crate::typed::PlaceProjection,
    ty: Ty,
) -> Option<crate::typed::PlaceId> {
    let (parent, base_version) = match typed.exprs.place_of(base).flatten() {
        Some(parent) => (parent, typed.exprs.place_version_of(base).flatten()?),
        None => ensure_temporary_owner(typed, env, base)?,
    };
    Some(projected_place_from_parent(
        typed,
        env,
        parent,
        base_version,
        projection,
        ty,
    ))
}

pub(super) fn ensure_temporary_owner(
    typed: &mut TypedFileAst,
    env: &FlowState,
    expression: ExprId,
) -> Option<(crate::typed::PlaceId, crate::typed::PlaceVersionId)> {
    if let Some(place) = typed.exprs.place_of(expression).flatten() {
        return Some((place, typed.exprs.place_version_of(expression).flatten()?));
    }
    if !temporary_value_is_owned(typed, env, expression) {
        return None;
    }
    let ty = typed.exprs.ty_of(expression)?.clone();
    let place = typed.add_place(crate::typed::TypedPlace {
        binder: ast::BinderId::new(typed.file_id.0, ast::BinderOwner::Expression(expression.0)),
        name: Intern::from_ref("temporary"),
        parent: None,
        projection: None,
        mutable: false,
        explicit_contract: true,
        ty: ty.clone(),
    });
    let version = typed.add_place_version(
        place,
        crate::typed::PlaceVersionOrigin::Temporary(expression),
        ty,
        typed
            .exprs
            .integer_knowledge_of(expression)
            .cloned()
            .flatten(),
    );
    typed.exprs.facts.ownership_facts.place[expression.index()] = Some(place);
    typed.exprs.facts.ownership_facts.place_version[expression.index()] = Some(version);
    assert_eq!(typed.temporary_owner(expression), Some(place));
    Some((place, version))
}

fn temporary_value_is_owned(typed: &TypedFileAst, env: &FlowState, expression: ExprId) -> bool {
    let Some(ty) = typed.exprs.ty_of(expression) else {
        return false;
    };
    if typed.borrows_value(expression)
        || !matches!(
            typed.type_registry.resolved_definition_for_type(ty),
            Ty::Record { .. } | Ty::Sequence(_) | Ty::Array { .. } | Ty::Union { .. }
        )
    {
        return false;
    }
    let mut source = expression;
    for _ in 0..typed.exprs.kind.len() {
        match &typed.exprs.kind[source.index()] {
            TypedExprKind::FnCall {
                target, args: None, ..
            } if env.locals.contains(&target.0) && !env.local_callables.contains_key(&target.0) => {
                return false;
            }
            TypedExprKind::AggregateGet { base, .. }
            | TypedExprKind::BufGet { buf: base, .. }
            | TypedExprKind::Cast { expr: base, .. }
            | TypedExprKind::Block {
                value: Some(base), ..
            } => source = *base,
            _ => break,
        }
    }
    true
}

fn projected_place_from_parent(
    typed: &mut TypedFileAst,
    env: &mut FlowState,
    parent: crate::typed::PlaceId,
    base_version: crate::typed::PlaceVersionId,
    projection: crate::typed::PlaceProjection,
    ty: Ty,
) -> crate::typed::PlaceId {
    let parent_place = typed.places[parent.0 as usize].clone();
    let place = env
        .projected_place(parent, &projection)
        .or_else(|| {
            typed.places.iter().enumerate().find_map(|(index, place)| {
                (place.parent == Some(parent) && place.projection.as_ref() == Some(&projection))
                    .then_some(crate::typed::PlaceId(index as u32))
            })
        })
        .unwrap_or_else(|| {
            typed.add_place(crate::typed::TypedPlace {
                binder: parent_place.binder,
                name: parent_place.name,
                parent: Some(parent),
                projection: Some(projection.clone()),
                mutable: parent_place.mutable,
                explicit_contract: true,
                ty: ty.clone(),
            })
        });
    env.set_projected_place(parent, projection, place);
    if env.projected_version(place).is_none() {
        let version = typed.add_place_version(
            place,
            crate::typed::PlaceVersionOrigin::Projection { base: base_version },
            ty,
            None,
        );
        env.set_projected_version(place, version);
    }
    place
}

fn current_version_for_place(
    typed: &TypedFileAst,
    env: &FlowState,
    place: crate::typed::PlaceId,
) -> Option<crate::typed::PlaceVersionId> {
    env.projected_version(place).or_else(|| {
        let place = typed.places.get(place.0 as usize)?;
        place
            .parent
            .is_none()
            .then(|| env.place_versions.get(&place.name).copied())
            .flatten()
    })
}

fn projected_value_evidence_for(
    kind: &TypedExprKind,
    typed: &TypedFileAst,
    env: &FlowState,
) -> crate::typed::ProjectedResultEvidence {
    match kind {
        TypedExprKind::Bind { body, .. }
        | TypedExprKind::Block {
            value: Some(body), ..
        } => typed
            .exprs
            .projected_result_evidence_of(*body)
            .cloned()
            .unwrap_or_default(),
        TypedExprKind::FnCall {
            target, args: None, ..
        } if env.locals.contains(&target.0) => env
            .projected_result_evidence
            .get(&target.0)
            .cloned()
            .unwrap_or_default(),
        TypedExprKind::ResultRow(items) | TypedExprKind::ArrayLit(items) => {
            let mut projected = crate::typed::ProjectedResultEvidence::new();
            for (index, item) in items.iter().enumerate() {
                if let Some(evidence) = typed.exprs.value_evidence_of(*item).cloned() {
                    projected.insert(vec![index], evidence);
                }
                if let Some(children) = typed.exprs.projected_result_evidence_of(*item) {
                    for (path, evidence) in children {
                        let mut projected_path = Vec::with_capacity(path.len() + 1);
                        projected_path.push(index);
                        projected_path.extend(path.iter().copied());
                        projected.insert(projected_path, evidence.clone());
                    }
                }
            }
            projected
        }
        TypedExprKind::AggregateGet { base, index } => typed
            .exprs
            .projected_result_evidence_of(*base)
            .into_iter()
            .flat_map(|evidence| evidence.iter())
            .filter(|(path, _)| path.first() == Some(index) && path.len() > 1)
            .map(|(path, evidence)| (path[1..].to_vec(), evidence.clone()))
            .collect(),
        _ => crate::typed::ProjectedResultEvidence::new(),
    }
}

fn value_evidence_for(
    kind: &TypedExprKind,
    ty: &Ty,
    typed: &TypedFileAst,
) -> Option<crate::typed::ValueEvidence> {
    if let TypedExprKind::FnCall {
        target,
        args: Some(args),
        ..
    } = kind
        && let Some(bind) = typed.defs.get(target)
        && let Some(returned) = match &bind.body {
            BindBody::Expr(expr)
            | BindBody::Body {
                ret: Some(expr), ..
            } => Some(*expr),
            BindBody::Body { ret: None, .. } | BindBody::Unlowered => None,
        }
        && let Some(TypedExprKind::FnCall {
            target: returned_parameter,
            args: None,
            ..
        }) = typed.exprs.kind_of(returned)
        && let Some(index) = bind
            .params
            .iter()
            .position(|(name, _)| name == &returned_parameter.0)
        && let Some(argument) = args.get(index)
        && let Some(evidence) = typed.exprs.value_evidence_of(*argument).cloned()
    {
        return Some(evidence);
    }
    if let TypedExprKind::FnCall {
        target,
        args: Some(args),
        ..
    } = kind
        && let Some(bind) = typed.defs.get(target)
        && !matches!(bind.body, crate::typed::BindBody::Unlowered)
        && let Ty::ResultFamily {
            owner,
            alternatives,
        } = &bind.return_type
    {
        let argument_for = |term: &ast::ProofTerm| {
            let ast::ProofTerm::Name(name) = term else {
                return None;
            };
            bind.params
                .iter()
                .position(|(parameter, _)| parameter == name)
                .and_then(|index| args.get(index).copied())
        };
        let evidence = alternatives
            .iter()
            .filter_map(|alternative| {
                let ast::ProofProposition::Compare {
                    left,
                    relation,
                    right,
                } = alternative.proposition.as_ref()?
                else {
                    return None;
                };
                let comparison = crate::typed::MathematicalComparison::from(*relation);
                Some(crate::typed::AlternativeEvidence {
                    label: alternative.label,
                    proposition: crate::typed::EvidenceProposition::IntegerComparison {
                        lhs: argument_for(left)?,
                        rhs: argument_for(right)?,
                        comparison,
                    },
                })
            })
            .collect::<Vec<_>>();
        if !evidence.is_empty() {
            return Some(crate::typed::ValueEvidence::Alternatives(
                crate::typed::ResultEvidence {
                    owner: owner.clone(),
                    alternatives: evidence,
                },
            ));
        }
    }
    if let TypedExprKind::FnCall {
        target,
        args: Some(args),
        ..
    } = kind
        && let Some(bind) = typed.defs.get(target)
        && !matches!(bind.body, crate::typed::BindBody::Unlowered)
        && let Some(evidence) = &bind.return_evidence
    {
        let substitute = |mut operand: ExprId| {
            while let Some(
                TypedExprKind::Deref(inner)
                | TypedExprKind::Ref(inner)
                | TypedExprKind::Rematerialize { source: inner, .. },
            ) = typed.exprs.kind_of(operand)
            {
                operand = *inner;
            }
            let TypedExprKind::FnCall {
                target: parameter,
                args: None,
                ..
            } = typed.exprs.kind_of(operand)?
            else {
                return Some(operand);
            };
            bind.params
                .iter()
                .position(|(name, _)| name == &parameter.0)
                .and_then(|index| args.get(index).copied())
                .or(Some(operand))
        };
        return evidence.map_operands(substitute);
    }
    let TypedExprKind::IntrinsicCall {
        op: IntrinsicOp::BitsCompare { predicate, .. },
        args,
    } = kind
    else {
        return None;
    };
    let [lhs, rhs] = args.as_slice() else {
        return None;
    };
    if !matches!(ty, Ty::Proposition(_)) {
        return None;
    }
    use crate::typed::MathematicalComparison as Math;
    let comparison = match predicate {
        crate::intrinsic::ComparisonPredicate::Less => Math::Less,
        crate::intrinsic::ComparisonPredicate::LessOrEqual => Math::LessOrEqual,
        crate::intrinsic::ComparisonPredicate::Greater => Math::Greater,
        crate::intrinsic::ComparisonPredicate::GreaterOrEqual => Math::GreaterOrEqual,
        crate::intrinsic::ComparisonPredicate::Equal => Math::Equal,
    };
    Some(crate::typed::ValueEvidence::Proposition(
        crate::typed::EvidenceProposition::IntegerComparison {
            lhs: *lhs,
            rhs: *rhs,
            comparison,
        },
    ))
}

fn masked_integer_knowledge(
    kind: &TypedExprKind,
    typed: &TypedFileAst,
) -> Option<ast::integer::IntegerKnowledge> {
    let TypedExprKind::IntrinsicCall {
        op: crate::intrinsic::IntrinsicOp::BitsAnd,
        args,
        ..
    } = kind
    else {
        return None;
    };
    args.iter()
        .filter_map(|arg| {
            let ast::integer::IntegerKnowledge::Exact(ast::integer::CanonicalIntegerExpr::Value(
                mask,
            )) = typed.exprs.integer_knowledge_of(*arg)?.as_ref()?
            else {
                return None;
            };
            (*mask >= i256::I256::from(0)).then_some(*mask)
        })
        .min()
        .and_then(|mask| ast::integer::IntegerDomain::bounded(i256::I256::from(0), mask))
        .map(ast::integer::IntegerKnowledge::Domain)
}

fn aggregate_projection_integer_knowledge(
    kind: &TypedExprKind,
    typed: &TypedFileAst,
    version: Option<crate::typed::PlaceVersionId>,
) -> Option<ast::integer::IntegerKnowledge> {
    let TypedExprKind::AggregateGet { base, index } = kind else {
        return None;
    };
    let item = aggregate_projection_initializer(typed, *base, *index, version?)?;
    typed.exprs.integer_knowledge_of(item)?.clone()
}

fn aggregate_projection_initializer(
    typed: &TypedFileAst,
    base: ExprId,
    index: usize,
    version: crate::typed::PlaceVersionId,
) -> Option<ExprId> {
    let crate::typed::PlaceVersionOrigin::Projection { base: source } =
        typed.place_versions[version.0 as usize].origin
    else {
        return None;
    };
    if typed.exprs.place_version_of(base).flatten() != Some(source) {
        return None;
    }
    let value = match typed.place_versions[source.0 as usize].origin {
        crate::typed::PlaceVersionOrigin::Initializer(value)
        | crate::typed::PlaceVersionOrigin::Rebind { value, .. } => value,
        _ => return None,
    };
    let value = aggregate_initializer(typed, value)?;
    match typed.exprs.kind_of(value)? {
        TypedExprKind::ResultRow(items)
        | TypedExprKind::ArrayLit(items)
        | TypedExprKind::RecordValue(items) => items.get(index).copied(),
        TypedExprKind::FixedArrayAlloc { init, size } => {
            let length = size.as_const_int()?;
            (i128::try_from(index).ok()? < length).then_some(*init)
        }
        _ => None,
    }
}

fn aggregate_initializer(typed: &TypedFileAst, value: ExprId) -> Option<ExprId> {
    match typed.exprs.kind_of(value)? {
        TypedExprKind::AggregateGet { base, index } => aggregate_projection_initializer(
            typed,
            *base,
            *index,
            typed.exprs.place_version_of(value).flatten()?,
        ),
        TypedExprKind::FnCall { args: None, .. } => {
            let version = typed.exprs.place_version_of(value).flatten()?;
            let value = match typed.place_versions[version.0 as usize].origin {
                crate::typed::PlaceVersionOrigin::Initializer(value)
                | crate::typed::PlaceVersionOrigin::Rebind { value, .. } => value,
                _ => return None,
            };
            aggregate_initializer(typed, value)
        }
        _ => Some(value),
    }
}

fn arithmetic_operand_type(typed: &TypedFileAst, operand: ExprId) -> Ty {
    let ty = &typed.exprs.ty[operand.index()];
    if ty.anonymous_integer_validity().is_some()
        && let Some(mut domain) = typed
            .exprs
            .integer_knowledge_of(operand)
            .and_then(Option::as_ref)
            .and_then(super::place::knowledge_domain)
    {
        let declared = ty.anonymous_integer_validity().unwrap().domain();
        if declared.prove_containment(&domain) == ast::integer::ContainmentProof::Proven {
            domain = declared.clone();
        } else if let Some(hull) = declared.storage_hull() {
            domain = domain.refine(ast::PredicateExpr::And(vec![
                ast::PredicateExpr::Ge(hull.min().into()),
                ast::PredicateExpr::Le(hull.max().into()),
            ]));
        }
        return Ty::AnonymousInteger {
            validity: ast::integer::IntegerValidity::new(domain),
        };
    }
    ty.clone()
}

fn integer_knowledge_for(
    ty: &Ty,
    const_value: Option<&ast::ConstValue>,
    registry: &crate::TypeRegistry,
) -> Option<ast::integer::IntegerKnowledge> {
    if let Some(ast::ConstValue::Int(value)) = const_value {
        return Some(ast::integer::IntegerKnowledge::Exact(
            ast::integer::CanonicalIntegerExpr::Value(*value),
        ));
    }
    if let Some(validity) = registry.integer_validity_for_type(ty) {
        return Some(ast::integer::IntegerKnowledge::Domain(
            validity.domain().clone(),
        ));
    }
    None
}

fn source_const_qualifier(expr: &ast::Expr) -> Option<String> {
    let (root, segments) = match expr {
        ast::Expr::FnCall(call) => (&call.path.value.root, &call.path.value.segments),
        ast::Expr::TagCall(call) => {
            let path = call.qual_path.as_ref()?.value.clone();
            return source_const_qualifier_from_path(&path.root, &path.segments);
        }
        _ => return None,
    };
    source_const_qualifier_from_path(root, segments)
}

fn source_const_qualifier_from_path(
    root: &Intern<String>,
    segments: &[Intern<String>],
) -> Option<String> {
    if segments.is_empty() {
        return None;
    }
    let mut path = root.as_str().to_string();
    for segment in segments.iter().take(segments.len().saturating_sub(1)) {
        path.push('.');
        path.push_str(segment.as_str());
    }
    Some(path)
}

fn materialized_integer_expected(
    expected: &Ty,
    registry: &crate::type_registry::TypeRegistry,
) -> Option<Ty> {
    if let Ty::Union { variants, .. } = registry.resolved_definition_for_type(expected) {
        let mut candidates = variants.iter().filter_map(|variant| {
            (variant.kind == ast::ty::UnionVariantKind::Type)
                .then(|| variant.fields.first().map(|(_, ty)| ty.as_ref()))
                .flatten()
                .filter(|ty| registry.integer_validity_for_type(ty).is_some())
        });
        let selected = candidates.next()?;
        return candidates.next().is_none().then(|| selected.clone());
    }
    match expected {
        Ty::UnresolvedLiteral(_) => None,
        _ => (!is_generic_opaque_ty(expected, registry)).then_some(expected.clone()),
    }
}

fn is_generic_opaque_ty(ty: &Ty, registry: &crate::type_registry::TypeRegistry) -> bool {
    matches!(
        registry.resolved_definition_for_type(ty),
        Ty::Opaque(name) if name.as_str().chars().next().is_some_and(char::is_lowercase)
    )
}

fn target_group_for_kind(
    kind: &TypedExprKind,
    typed: &TypedFileAst,
    scope: &ExprLowerScope<'_>,
    env: &FlowState,
    _flaws: &mut Vec<Diagnostic>,
) -> Option<ReferenceTargetSet> {
    let child_group = |id: ExprId| typed.exprs.target_group_of(id).cloned().flatten();

    match kind {
        TypedExprKind::Block {
            value: Some(value), ..
        } => child_group(*value),
        TypedExprKind::When(branch) => {
            let mut targets = ReferenceTargetSet::default();
            for arm in &branch.arms {
                let body = match arm {
                    TypedWhenArm::Cond { body, .. }
                    | TypedWhenArm::Is { body, .. }
                    | TypedWhenArm::Else(body, _) => *body,
                };
                if !crate::analysis::expression_returns(typed.resolved_program(), body) {
                    if let Some(group) = child_group(body) {
                        targets.extend(&group);
                    }
                }
            }
            let has_targets = targets.iter().next().is_some();
            has_targets.then_some(targets)
        }
        TypedExprKind::SelfRef { .. } => env.target_groups.get(&Intern::from_ref("self")).cloned(),
        TypedExprKind::FnCall { target, args, .. } if args.as_ref().is_none_or(Vec::is_empty) => {
            env.target_groups.get(&target.0).cloned()
        }
        TypedExprKind::FnCall {
            target,
            args: Some(args),
            ..
        } => {
            let local_signature = typed.defs.get(target).map(TypedCallableSignature::from);
            let signature = local_signature
                .as_ref()
                .or_else(|| scope.callable_signatures.get(target))?;
            let return_group = signature.result_groups.first().copied().flatten()?;
            typed
                .apply_target_groups(args, signature)
                .targets(return_group)
                .cloned()
        }
        TypedExprKind::Ref(inner) | TypedExprKind::TakePtr(inner) => child_group(*inner),
        TypedExprKind::Reassign { name, .. } => env.target_groups.get(name).cloned(),
        TypedExprKind::Bind { name, .. } => env.target_groups.get(name).cloned(),
        TypedExprKind::Deref(inner) => child_group(*inner).map(|targets| {
            targets.projected(|target| ReferenceTargetGroup::Deref(Box::new(target.clone())))
        }),
        TypedExprKind::AggregateGet { base, index } => typed
            .exprs
            .result_target_groups_of(*base)
            .and_then(|fields| fields.get(*index))
            .and_then(Clone::clone)
            .or_else(|| {
                child_group(*base).map(|targets| {
                    targets.projected(|target| ReferenceTargetGroup::Field {
                        base: Box::new(target.clone()),
                        index: *index,
                    })
                })
            }),
        TypedExprKind::SliceGet { buf, .. } => child_group(*buf).map(|targets| {
            targets.projected(|target| ReferenceTargetGroup::ItemRegion {
                base: Box::new(target.clone()),
                index: crate::typed::TargetIndex::Unknown,
            })
        }),
        TypedExprKind::BufGet { buf, index } => {
            let index = target_index_for_expr(typed, *index);
            child_group(*buf).map(|targets| {
                targets.projected(|target| ReferenceTargetGroup::ItemRegion {
                    base: Box::new(target.clone()),
                    index: index.clone(),
                })
            })
        }
        _ => None,
    }
}

fn result_target_groups_for_kind(
    kind: &TypedExprKind,
    typed: &TypedFileAst,
    scope: &ExprLowerScope<'_>,
    env: &FlowState,
) -> Vec<Option<ReferenceTargetSet>> {
    if let TypedExprKind::Block {
        value: Some(value), ..
    } = kind
    {
        return typed
            .exprs
            .result_target_groups_of(*value)
            .cloned()
            .unwrap_or_default();
    }
    let fields = match kind {
        TypedExprKind::ResultRow(fields) => Some(fields.as_slice()),
        TypedExprKind::RecordValue(fields) => Some(fields.as_slice()),
        TypedExprKind::TagCall {
            args: Some(fields), ..
        } => Some(fields.as_slice()),
        _ => None,
    };
    if let Some(fields) = fields {
        return fields
            .iter()
            .map(|field| typed.exprs.target_group_of(*field).cloned().flatten())
            .collect();
    }
    let TypedExprKind::FnCall { target, args, .. } = kind else {
        return Vec::new();
    };
    if args.is_none() {
        return env
            .result_target_groups
            .get(&target.0)
            .cloned()
            .unwrap_or_default();
    }
    let Some(args) = args else { return Vec::new() };
    let Some(signature) = typed
        .defs
        .get(target)
        .map(TypedCallableSignature::from)
        .or_else(|| scope.callable_signatures.get(target).cloned())
    else {
        return Vec::new();
    };
    let application = typed.apply_target_groups(args, &signature);
    signature
        .result_groups
        .iter()
        .map(|group| group.and_then(|group| application.targets(group).cloned()))
        .collect()
}

pub(crate) fn target_index_for_expr(typed: &TypedFileAst, expr_id: ExprId) -> TargetIndex {
    let idx = expr_id.as_usize();
    if let Some(value) = typed
        .exprs
        .facts
        .const_facts
        .const_value
        .get(idx)
        .and_then(Clone::clone)
    {
        return TargetIndex::Symbolic(ast::NormalExpr::Value(value));
    }
    let Some(kind) = typed.exprs.kind.get(idx) else {
        return TargetIndex::Unknown;
    };
    let expr = match kind {
        TypedExprKind::Lit(Literal::Int(value)) => {
            ast::NormalExpr::Value(ast::ConstValue::Int(*value))
        }
        TypedExprKind::Ref(inner) | TypedExprKind::TakePtr(inner) | TypedExprKind::Deref(inner) => {
            return target_index_for_expr(typed, *inner);
        }
        TypedExprKind::Cast { expr, .. } => return target_index_for_expr(typed, *expr),
        TypedExprKind::Rematerialize { source, .. } => {
            return target_index_for_expr(typed, *source);
        }
        TypedExprKind::TargetQuery { kind, operand } => {
            let Some(operand) = crate::subst::type_reference_for_ty(operand) else {
                return TargetIndex::Unknown;
            };
            ast::NormalExpr::TargetQuery {
                kind: *kind,
                operand,
            }
        }
        TypedExprKind::FnCall { target, args, .. } if args.as_ref().is_none_or(Vec::is_empty) => {
            ast::NormalExpr::Var(target.0)
        }

        _ => return TargetIndex::Unknown,
    };
    TargetIndex::Symbolic(expr)
}

fn pattern_surface(pattern: &ast::Pattern) -> String {
    match pattern {
        ast::Pattern::Invalid => "<invalid>".to_string(),
        ast::Pattern::Nominal(name, _) => name.as_str().to_string(),
        ast::Pattern::Qualified(path) => {
            let mut out = path.value.root.as_str().to_string();
            for segment in &path.value.segments {
                out.push('.');
                out.push_str(segment.as_str());
            }
            out
        }
        ast::Pattern::Generic {
            name,
            params,
            param_spans,
            ..
        } => {
            let params = param_spans
                .iter()
                .map(|(param_name, _)| {
                    let Some((_, kind)) = params.iter().find(|(name, _)| name == param_name) else {
                        return format!("<invalid-argument:{}>", param_name.as_str());
                    };
                    match kind {
                        ast::ParameterKind::Proposition(predicate) => {
                            format!("{param_name} is {predicate}")
                        }
                        ast::ParameterKind::Generic => param_name.as_str().to_string(),
                        ast::ParameterKind::Tagged(sp) => {
                            let surface = sp.value.format_surface();
                            if surface == param_name.as_str() {
                                surface
                            } else {
                                format!("{} {}", param_name.as_str(), surface)
                            }
                        }
                        ast::ParameterKind::ValueParam { ty } => {
                            format!("{} {}", param_name.as_str(), ty.value.format_surface())
                        }
                        ast::ParameterKind::Inferred { ty } => {
                            format!("{} {}: ?", param_name.as_str(), ty.value.format_surface())
                        }
                        ast::ParameterKind::Default(expr) => {
                            format!("{}: {:?}", param_name.as_str(), expr.value)
                        }
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", name.as_str(), params)
        }
        ast::Pattern::Literal(lit, _) => lit.to_string(),
        ast::Pattern::Pointer(inner) => format!("@{}", pattern_surface(&inner.value)),
        ast::Pattern::Ref { inner, mutable, .. } => format!(
            "{}{}",
            if *mutable { "mut " } else { "ref " },
            pattern_surface(&inner.value)
        ),
        ast::Pattern::Unit => "()".to_string(),
        ast::Pattern::ListEmpty => "[]".to_string(),
        ast::Pattern::ListCons { head, tail } => {
            format!(
                "[{}, ...{}]",
                pattern_surface(&head.value),
                pattern_surface(&tail.value)
            )
        }
        ast::Pattern::InRange { bounds, .. } => format!("in {bounds}"),
    }
}

fn when_pattern_is_wildcard(pattern: &ast::Pattern) -> bool {
    matches!(pattern, ast::Pattern::Nominal(name, _) if name.as_str() == "_")
}

fn const_for_def_id(
    def_id: DefId,
    typed: &TypedFileAst,
    constants: &super::constant_queries::ConstantQueries,
    reads: Option<&super::implementation_reads::ImplementationReadRecorder>,
) -> Option<ast::ConstValue> {
    let Some(bind) = typed.defs.get(&def_id) else {
        let answer = constants.value(def_id);
        if let Some(reads) = reads {
            reads.constant(def_id, &answer);
        }
        return answer;
    };
    let BindBody::Expr(body) = bind.body else {
        return None;
    };
    let mut evaluator = crate::eval::Evaluator::new(
        typed.resolved_program(),
        crate::eval::CompileTimeHost::new(&flask::CompileTarget::Library),
    );
    if let Ok(crate::eval::EvalOutcome::Complete(value)) = evaluator.eval(body) {
        return Some(value);
    }
    typed
        .exprs
        .facts
        .const_facts
        .const_value
        .get(body.as_usize())?
        .clone()
        .or_else(|| match typed.exprs.kind.get(body.as_usize())? {
            TypedExprKind::TagCall {
                variant_id, args, ..
            } => {
                let args = args
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|arg| typed.exprs.const_value_of(*arg).cloned().flatten())
                    .collect::<Option<Vec<_>>>()?;
                Some(ast::ConstValue::Tag {
                    name: variant_id.name,
                    qual_path: None,
                    args: args.into(),
                })
            }
            _ => None,
        })
}

/// Try to extract a [`ConstValue`] from a pattern parameter's [`ParameterKind`].
fn param_kind_const(kind: &ParameterKind) -> Option<ast::ConstValue> {
    match kind {
        ParameterKind::Tagged(sp)
        | ParameterKind::ValueParam { ty: sp }
        | ParameterKind::Inferred { ty: sp } => match &sp.value {
            ast::Expr::Lit(lit) => Some(crate::const_value::const_value_of_literal(lit)),
            _ => None,
        },
        ParameterKind::Proposition(_) => None,
        ParameterKind::Generic | ParameterKind::Default(_) => None,
    }
}

pub(crate) fn pattern_value_const(
    pattern: &ast::Pattern,
    typed: &TypedFileAst,
) -> Option<ast::ConstValue> {
    // Variant-based resolution for uppercase nominal and qualified patterns
    match pattern {
        // Uppercase nominal: likely a variant tag name (e.g., `True`, `False`)
        ast::Pattern::Nominal(name, _)
            if !name.as_str().starts_with_lowercase() && name.as_str() != "_" =>
        {
            if typed.variant_map.contains_key(name) {
                return Some(ast::ConstValue::Tag {
                    name: *name,
                    qual_path: None,
                    args: Vec::new().into(),
                });
            }
        }
        // Qualified path: e.g., `Bool.True` → variant "True" of union "Bool"
        ast::Pattern::Qualified(path) => {
            if let Some(variant_name) = path.segments.last()
                && typed.variant_map.contains_key(variant_name)
            {
                return Some(ast::ConstValue::Tag {
                    name: *variant_name,
                    qual_path: None,
                    args: Vec::new().into(),
                });
            }
        }
        ast::Pattern::Generic {
            name,
            params,
            param_spans,
            ..
        } if typed.variant_map.contains_key(name) => {
            let args = param_spans
                .iter()
                .map(|(param_name, _)| {
                    params
                        .iter()
                        .find(|(name, _)| name == param_name)
                        .and_then(|(_, kind)| param_kind_const(kind))
                })
                .collect::<Option<Vec<_>>>()?;
            return Some(ast::ConstValue::Tag {
                name: *name,
                qual_path: None,
                args: args.into(),
            });
        }
        _ => {}
    }
    None
}

fn patterns_match_variant_with_values(
    patterns: &[&ast::Pattern],
    variant: &ast::ty::UnionVariant,
    typed: &TypedFileAst,
) -> bool {
    crate::analysis::patterns_cover_union_variant(patterns, variant, Some(&typed.type_registry))
        || (variant.fields.is_empty()
            && patterns.iter().any(|pattern| matches!(
                pattern_value_const(pattern, typed),
                Some(ast::ConstValue::Tag { name, args, .. }) if name == variant.name && args.is_empty()
            )))
}

fn pattern_matches_const_with_values(
    pattern: &ast::Pattern,
    value: &ast::ConstValue,
    typed: &TypedFileAst,
) -> bool {
    crate::analysis::pattern::pattern_matches_with_tag_types(
        pattern,
        value,
        &typed.tag_types,
        &typed.type_registry,
    ) || pattern_value_const(pattern, typed)
        .as_ref()
        .is_some_and(|pattern_value| pattern_value == value)
}

fn when_is_exhaustive_with_value_patterns(
    subject_ty: Option<&Ty>,
    arms: &[TypedWhenArm],
    typed: &TypedFileAst,
) -> bool {
    let Some(subject_ty) = subject_ty else {
        return false;
    };
    let subject_ty = reference_pointee_for_type(subject_ty, Some(&typed.type_registry))
        .unwrap_or_else(|| subject_ty.clone());
    let patterns: Vec<_> = crate::analysis::is_patterns_from_arms(arms).collect();
    if patterns.is_empty() {
        return false;
    }
    if patterns
        .iter()
        .any(|pattern| pattern.is_catch_all_pattern())
    {
        return true;
    }
    let subject_definition = typed
        .type_registry
        .resolved_definition_for_type(&subject_ty);
    if when_is_exhaustive(Some(&subject_definition), arms, &typed.type_registry) {
        return true;
    }
    match &subject_definition {
        ty if ty.union_literal_values().is_some() => {
            let Some(values) = subject_definition.union_literal_values() else {
                return false;
            };
            values.iter().all(|value| {
                patterns
                    .iter()
                    .any(|pattern| pattern_matches_const_with_values(pattern, value, typed))
            })
        }
        Ty::Union { variants, .. } => variants
            .iter()
            .all(|v| patterns_match_variant_with_values(&patterns, v, typed)),
        _ => when_is_exhaustive(Some(&subject_definition), arms, &typed.type_registry),
    }
}

fn validate_when_arm_shape(
    arms: &[TypedWhenArm],
    has_subject: bool,
    typed: &TypedFileAst,
    flaws: &mut Vec<Diagnostic>,
) {
    let mut seen_else = false;
    let mut seen_is = false;
    let mut seen_catch_all = false;
    let mut covered_patterns = HashSet::new();
    let mut covered_values: Vec<ast::ConstValue> = Vec::new();
    for arm in arms {
        match arm {
            TypedWhenArm::Else(..) => {
                seen_else = true;
            }
            TypedWhenArm::Cond { .. } => {
                if seen_else {
                    // type-unreachable-when-arm
                    flaws.push(Diagnostic::new(
                        PatternCode::ArmUnreachable,
                        "when arm is unreachable",
                    ));
                }
            }
            TypedWhenArm::Is { pattern, .. } => {
                if seen_else || seen_catch_all {
                    // type-unreachable-when-arm
                    flaws.push(Diagnostic::new(
                        PatternCode::ArmUnreachable,
                        "when arm is unreachable",
                    ));
                }
                seen_is = true;
                if when_pattern_is_wildcard(&pattern.value) {
                    // type-wildcard-when-pattern
                    flaws.push(Diagnostic::new(
                        PatternCode::Wildcard,
                        "wildcard `_` is not allowed in when patterns",
                    ));
                }
                seen_catch_all |= pattern.value.is_catch_all_pattern();
                // Shape-based duplicate detection
                let key = pattern_surface(&pattern.value);
                if !covered_patterns.insert(key.clone()) {
                    // type-unreachable-when-arm
                    flaws.push(Diagnostic::new(
                        PatternCode::ArmUnreachable,
                        "when arm is unreachable",
                    ));
                } else if let Some(cv) = pattern_value_const(&pattern.value, typed) {
                    // Semantic duplicate detection: same resolved value
                    if covered_values.iter().any(|v| v == &cv) {
                        // type-unreachable-when-arm
                        flaws.push(Diagnostic::new(
                            PatternCode::ArmUnreachable,
                            "when arm is unreachable",
                        ));
                    } else {
                        covered_values.push(cv);
                    }
                }
            }
        }
    }
    if seen_else && !matches!(arms.last(), Some(TypedWhenArm::Else(..))) {
        // type-else-not-last
        flaws.push(Diagnostic::new(
            PatternCode::ElseNotLast,
            "`else` must be the last arm in `when`",
        ));
    }
    if seen_is && !has_subject {
        // type-mixed-when-forms
        flaws.push(Diagnostic::new(
            PatternCode::MixedWhenForms,
            "cannot mix condition and pattern arms in `when`",
        ));
    }
}

/// Validate named/positional fields in a record-literal TagCall.
///
/// Checks for:
/// - Duplicate field names
/// - Unknown field names
/// - Missing required fields
/// - Type mismatches between arg expressions and declared field types
fn validate_record_fields(
    tag_name: &Intern<String>,
    record_fields: &[(Intern<String>, Box<Ty>)],
    field_names: &[Intern<String>],
    arg_ids: Option<&[ExprId]>,
    typed: &TypedFileAst,
    flaws: &mut Vec<Diagnostic>,
) {
    use std::collections::HashSet;

    let valid_field_set: HashSet<&Intern<String>> = record_fields.iter().map(|(n, _)| n).collect();

    // 1. Duplicate field detection
    let mut seen = HashSet::new();
    for name in field_names {
        if name.as_str().is_empty() {
            continue; // positional arg, handled below
        }
        if !seen.insert(name) {
            flaws.push(
                // type-duplicate-field
                Diagnostic::new(
                    MemberCode::Duplicate,
                    format!("field `{}` provided twice", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            );
        }
    }

    // 2. Unknown field detection
    for name in field_names.iter() {
        if name.as_str().is_empty() {
            // type-expected-field-name
            flaws.push(Diagnostic::new(
                MemberCode::ExpectedName,
                "expected field name or `name: expr`".to_string(),
            ));
            continue;
        }
        if !valid_field_set.contains(name) {
            flaws.push(
                // type-unknown-field
                Diagnostic::new(
                    MemberCode::Unknown,
                    format!("`{}` has no field `{}`", tag_name.as_str(), name.as_str()),
                )
                .with_arg("name", name.as_str().to_string())
                .with_arg("tag", tag_name.as_str().to_string()),
            );
        }
    }

    // 3. Missing field detection
    let provided_names: HashSet<&Intern<String>> = field_names
        .iter()
        .filter(|n| !n.as_str().is_empty())
        .collect();
    for (field_name, _) in record_fields {
        if !provided_names.contains(field_name) {
            flaws.push(
                // type-missing-field
                Diagnostic::new(
                    MemberCode::Missing,
                    format!(
                        "missing field `{}` in `{}(...)`",
                        field_name.as_str(),
                        tag_name.as_str()
                    ),
                )
                .with_arg("name", field_name.as_str().to_string())
                .with_arg("tag", tag_name.as_str().to_string()),
            );
        }
    }

    // 4. Type mismatch checks
    if let Some(args) = arg_ids {
        for (i, name) in field_names.iter().enumerate() {
            if name.as_str().is_empty() {
                continue;
            }
            if let Some(arg_id) = args.get(i) {
                let arg_ty = typed.exprs.ty.get(arg_id.as_usize());
                // Find the declared field type
                if let Some((_, field_ty)) = record_fields.iter().find(|(n, _)| *n == *name)
                    && let Some(arg_ty) = arg_ty
                    && types_are_incompatible(arg_ty, field_ty)
                {
                    flaws.push(
                        // type-mismatch
                        Diagnostic::new(
                            TypeCode::Mismatch,
                            format!("type mismatch for field `{}`", name.as_str(),),
                        )
                        .with_arg("name", name.as_str().to_string()),
                    );
                }
            }
        }
    }
}

/// Check if two types are structurally incompatible (for basic type mismatch detection).
/// Returns `true` if they are definitely incompatible.
fn types_are_incompatible(arg_ty: &Ty, field_ty: &Ty) -> bool {
    match (arg_ty, field_ty) {
        // String literal assigned to Int field
        (Ty::Literal(ConstValue::Text(_)), Ty::AnonymousInteger { .. }) => true,
        // Int literal assigned to non-Int field like Bool (union)
        (Ty::Literal(ConstValue::Int(_)), Ty::Union { .. }) => true,
        // String literal assigned to union (Bool is True/False)
        (Ty::Literal(ConstValue::Text(_)), Ty::Union { .. }) => true,
        (Ty::Opaque(actual), Ty::Opaque(expected)) => actual != expected,
        _ => false,
    }
}

struct FlawContext<'a, 'f> {
    typed: &'a TypedFileAst,
    name_span_id: Option<SpanId>,
    locals: &'a HashSet<Intern<String>>,
    local_var_types: &'a LocalVarTypes,
    flaws: &'f mut Vec<Diagnostic>,
    value_required: bool,
}

fn check_type_flaws(expr_id: ExprId, kind: &TypedExprKind, ty: &Ty, context: FlawContext<'_, '_>) {
    let FlawContext {
        typed,
        name_span_id,
        locals,
        local_var_types,
        flaws,
        value_required,
    } = context;
    match kind {
        TypedExprKind::FixedArrayAlloc { init, .. } => {
            if let Some((element, _)) =
                crate::representation::Repr::array_parts(ty, Some(&typed.type_registry))
                && let Some(initializer) = typed.exprs.ty.get(init.as_usize())
                && initializer != &element
                && !matches!(initializer, Ty::UnresolvedLiteral(_))
            {
                // type-fixed-array-element-mismatch
                flaws.push(Diagnostic::new(
                    ArrayCode::FixedElementMismatch,
                    "fixed-array initializer does not match the selected element type",
                ));
            }
        }
        TypedExprKind::ArrayLit(items) => {
            if let Some((element, _)) =
                crate::representation::Repr::array_parts(ty, Some(&typed.type_registry))
            {
                for initializer in items
                    .iter()
                    .filter_map(|item| typed.exprs.ty.get(item.as_usize()))
                {
                    if initializer != &element && !matches!(initializer, Ty::UnresolvedLiteral(_)) {
                        // type-fixed-array-element-mismatch
                        flaws.push(Diagnostic::new(
                            ArrayCode::FixedElementMismatch,
                            "fixed-array element does not match the selected element type",
                        ));
                        break;
                    }
                }
            }
        }
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            let is_local_callable = typed.local_callable_calls.contains_key(&expr_id);
            if !is_local_callable {
                check_fn_call_bounds(
                    typed,
                    target,
                    args.as_deref(),
                    substituted_ty.as_ref(),
                    flaws,
                );
            }
            let is_known = is_local_callable
                || typed.defs.contains_key(target)
                || typed.fn_return_types.contains_key(target)
                || locals.contains(&target.0)
                || local_var_types.contains_key(&target.0);
            if !is_known {
                let name = target.0.as_str();
                let did_you_mean = closest_name(
                    name,
                    typed
                        .defs
                        .keys()
                        .map(|d| d.0.as_str())
                        .chain(typed.fn_return_types.keys().map(|d| d.0.as_str()))
                        .chain(typed.tags.keys().map(|t| t.0.as_str()))
                        .chain(locals.iter().map(|l| l.as_str()))
                        .chain(local_var_types.keys().map(|t| t.as_str())),
                );
                let mut diag =
                    // type-unknown-symbol
                    Diagnostic::new(SymbolCode::Unknown, format!("unknown symbol `{}`", name))
                        .with_arg("name", name.to_string());
                if let Some(name_span) = name_span_id {
                    diag = diag.at_span_id(name_span, &typed.span_table);
                }
                if let Some(suggestion) = &did_you_mean {
                    diag = diag.with_help(format!("did you mean `{}`?", suggestion));
                }
                flaws.push(diag);
            }
        }
        TypedExprKind::TagCall {
            variant_id,
            field_names,
            args,
            ..
        } => {
            let is_record = is_record_tag_call(variant_id, ty, typed);

            // Unknown symbol: bare tag that isn't a known record
            let is_result_alternative = matches!(
                ty,
                Ty::ResultFamily { alternatives, .. }
                    if alternatives.iter().any(|alternative| alternative.label == variant_id.name)
            );
            if variant_id.union.0 == variant_id.name && !is_record && !is_result_alternative {
                // type-unknown-symbol
                let mut diag = Diagnostic::new(
                    SymbolCode::Unknown,
                    format!("unknown symbol `{}`", variant_id.name.as_str()),
                )
                .with_arg("name", variant_id.name.as_str().to_string());
                if let Some(name_span) = name_span_id {
                    diag = diag.at_span_id(name_span, &typed.span_table);
                }
                flaws.push(diag);
            }

            // Field validation for record types (shape literals)
            if is_record
                && typed
                    .tags
                    .get(&TagId(variant_id.name))
                    .is_none_or(|tag| tag.record_field_refinements.is_empty())
                && let Ty::Record { fields, .. } =
                    typed.type_registry.resolved_definition_for_type(ty)
            {
                validate_record_fields(
                    &variant_id.name,
                    &fields,
                    field_names,
                    args.as_deref(),
                    typed,
                    flaws,
                );
            }
        }
        TypedExprKind::Binary { op, lhs, rhs } => {
            let lhs_ty = typed.exprs.ty.get(lhs.as_usize());
            let rhs_ty = typed.exprs.ty.get(rhs.as_usize());
            if let (Some(lhs_ty), Some(rhs_ty)) = (lhs_ty, rhs_ty) {
                if *op == BinOp::Equal {
                    if let Err(error) = crate::equality::validate_with_registry(
                        lhs_ty,
                        rhs_ty,
                        &typed.type_registry,
                    ) {
                        let (code, message) = match error {
                            crate::equality::EqualityError::DifferentTypes => (
                                DiagnosticCode::Type(TypeCode::Operator(
                                    // type-equality-mismatched-operands
                                    OperatorCode::EqualityMismatchedOperands,
                                )),
                                format!(
                                    "equality requires operands of the same type, found `{}` and `{}`",
                                    lhs_ty.format_for_hover(),
                                    rhs_ty.format_for_hover()
                                ),
                            ),
                            crate::equality::EqualityError::UnsupportedType => (
                                DiagnosticCode::Type(TypeCode::Operator(
                                    // type-equality-unsupported
                                    OperatorCode::EqualityUnsupported,
                                )),
                                format!(
                                    "equality is not supported for `{}`",
                                    lhs_ty.format_for_hover()
                                ),
                            ),
                        };
                        flaws.push(
                            Diagnostic::new(code, message)
                                .with_arg("left_type", lhs_ty.format_for_hover())
                                .with_arg("right_type", rhs_ty.format_for_hover()),
                        );
                    }
                    return;
                }
                let lhs_is_int = typed
                    .type_registry
                    .integer_validity_for_type(lhs_ty)
                    .is_some();
                let rhs_is_int = typed
                    .type_registry
                    .integer_validity_for_type(rhs_ty)
                    .is_some();
                let lhs_is_float = lhs_ty.is_float();
                let rhs_is_float = rhs_ty.is_float();
                if (lhs_is_int && rhs_is_float) || (lhs_is_float && rhs_is_int) {
                    // type-mismatch
                    flaws.push(Diagnostic::new(TypeCode::Mismatch, "type mismatch"));
                }
            }
        }
        TypedExprKind::When(when_expr) => {
            validate_when_arm_shape(&when_expr.arms, when_expr.subject.is_some(), typed, flaws);
            let arm_types = when_expr.arms.iter().filter_map(|arm| {
                let body = match arm {
                    TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. } => *body,
                    TypedWhenArm::Else(body, _) => *body,
                };
                (!crate::analysis::expression_returns(typed.resolved_program(), body))
                    .then(|| typed.exprs.ty.get(body.as_usize()))
                    .flatten()
            });
            if let Some(Ty::Sequence(first)) =
                arm_types.clone().find(|ty| matches!(ty, Ty::Sequence(_)))
                && arm_types
                    .filter_map(|ty| match ty {
                        Ty::Sequence(elements) => Some(elements),
                        _ => None,
                    })
                    .any(|elements| {
                        first
                            .iter()
                            .map(|element| element.name)
                            .ne(elements.iter().map(|element| element.name))
                    })
            {
                // type-incompatible-result-names
                flaws.push(Diagnostic::new(
                    ResultCode::NamesIncompatible,
                    "when arms must produce compatible named results in the same order",
                ));
            }
            let has_else = when_expr
                .arms
                .iter()
                .any(|arm| matches!(arm, TypedWhenArm::Else(..)));
            let subject_ty = when_expr
                .subject
                .and_then(|id| typed.exprs.ty.get(id.as_usize()));
            let exhaustive =
                when_is_exhaustive_with_value_patterns(subject_ty, &when_expr.arms, typed);
            if value_required && !has_else && !exhaustive {
                // type-missing-else-arm
                flaws.push(Diagnostic::new(
                    PatternCode::ElseMissing,
                    "`when` should have an `else` arm",
                ));
            } else if has_else && exhaustive {
                // type-unreachable-else-arm
                flaws.push(Diagnostic::new(
                    PatternCode::ElseUnreachable,
                    "`else` arm is unreachable",
                ));
            }
            // Subject-value reachability: if the subject has a known compile-time value,
            // check which arms match and flag unmatched/after-match arms as unreachable.
            if let Some(subject_id) = when_expr.subject
                && !matches!(
                    typed.exprs.kind_of(subject_id),
                    Some(TypedExprKind::TargetField(_))
                )
                && let Some(Some(subject_cv)) = typed
                    .exprs
                    .facts
                    .const_facts
                    .const_value
                    .get(subject_id.as_usize())
            {
                let mut matching_arm_found = false;
                for arm in &when_expr.arms {
                    match arm {
                        TypedWhenArm::Is { pattern, .. } => {
                            if matching_arm_found {
                                // type-unreachable-when-arm
                                flaws.push(Diagnostic::new(
                                    PatternCode::ArmUnreachable,
                                    "when arm is unreachable",
                                ));
                            } else if let Some(pattern_cv) =
                                pattern_value_const(&pattern.value, typed)
                            {
                                // Pattern resolves to a specific const value
                                if &pattern_cv != subject_cv {
                                    // type-unreachable-when-arm
                                    flaws.push(Diagnostic::new(
                                        PatternCode::ArmUnreachable,
                                        "when arm is unreachable",
                                    ));
                                } else {
                                    matching_arm_found = true;
                                }
                            } else if !crate::analysis::pattern::pattern_matches_with_tag_types(
                                &pattern.value,
                                subject_cv,
                                &typed.tag_types,
                                &typed.type_registry,
                            ) {
                                // Pattern can be statically compared against subject value
                                // (handles literal, InRange, uppercase variant patterns)
                                // type-unreachable-when-arm
                                flaws.push(Diagnostic::new(
                                    PatternCode::ArmUnreachable,
                                    "when arm is unreachable",
                                ));
                            } else {
                                matching_arm_found = true;
                            }
                        }
                        TypedWhenArm::Else(..) => {
                            if matching_arm_found {
                                // type-unreachable-else-arm
                                flaws.push(Diagnostic::new(
                                    PatternCode::ElseUnreachable,
                                    "`else` arm is unreachable",
                                ));
                            }
                        }
                        TypedWhenArm::Cond { .. } => {}
                    }
                }
            }
        }
        TypedExprKind::Cast { expr, ty } => {
            let source = typed.exprs.ty.get(expr.as_usize());
            let discriminant_conversion = source.is_some_and(|source| {
                crate::discriminant::is_numeric_cast(&typed.type_registry, source, ty)
            });
            flaws.extend(crate::discriminant::validate_cast(typed, *expr, ty));
            if source.is_some_and(|source| {
                source != ty
                    && !discriminant_conversion
                    && matches!(
                        typed.type_registry.resolved_definition_for_type(source),
                        Ty::Union { .. }
                    )
            }) {
                // type-incompatible-union-value
                flaws.push(Diagnostic::new(
                    UnionCode::ValueIncompatible,
                    "exclude the other union alternatives before converting its member",
                ));
            }
            let source_space = source.and_then(Ty::address_space);
            let target_space = ty.address_space();
            let source_width = source.and_then(|ty| typed.type_registry.integer_width_for_type(ty));
            let target_width = typed.type_registry.integer_width_for_type(ty);
            if let (Some(source), Some(expected)) =
                (source, typed.type_registry.integer_validity_for_type(ty))
                && typed
                    .type_registry
                    .integer_validity_for_type(source)
                    .is_some()
            {
                let actual = super::place::knowledge_domain_or(
                    typed.exprs.facts.semantic_facts.integer_knowledge[expr.as_usize()].as_ref(),
                    || {
                        typed
                            .type_registry
                            .integer_validity_for_type(source)
                            .map(|validity| validity.domain().clone())
                    },
                );
                let proof = actual
                    .as_ref()
                    .map_or(ast::integer::ContainmentProof::Unknown, |actual| {
                        actual.prove_containment(expected.domain())
                    });
                let target_dependent = typed.type_registry.is_target_dependent_integer(source)
                    || typed.type_registry.is_target_dependent_integer(ty);
                if let Some(code) = proof.diagnostic_code()
                    && !(proof == ast::integer::ContainmentProof::Unknown && target_dependent)
                {
                    flaws.push(
                        Diagnostic::new(
                            code,
                            "integer conversion requires the source domain to be contained by the target validity",
                        )
                        .with_arg("source", format!("{source:?}"))
                        .with_arg("target", format!("{ty:?}")),
                    );
                }
            }
            if let Some(address_space) = source_space.or(target_space)
                && address_space > u8::MAX as u32
            {
                flaws.push(
                    // type-unsupported-address-space
                    Diagnostic::new(
                        AddressCode::SpaceUnsupported,
                        "raw address conversion uses an unsupported address space",
                    )
                    .with_arg("address_space", address_space.to_string()),
                );
            }
            if source_space.is_some() && target_width.is_some()
                || target_space.is_some() && source_width.is_some()
            {
                let width = target_width.or(source_width).unwrap_or_default();
                if !matches!(width, 32 | 64) {
                    flaws.push(
                        // type-invalid-address-conversion-width
                        Diagnostic::new(
                            AddressCode::ConversionWidthInvalid,
                            "raw address conversion requires a 32-bit or 64-bit integer",
                        )
                        .with_arg("width", width.to_string()),
                    );
                }
            }
        }
        TypedExprKind::TakePtr(inner) => {
            let pointee = typed.exprs.ty.get(inner.as_usize());
            let resolved_ty = typed.type_registry.resolved_definition_for_type(ty);
            if let Some(address_space) = resolved_ty.address_space() {
                if let Some(expected_pointee) = resolved_ty.pointee_ty()
                    && pointee.is_some_and(|actual| actual != expected_pointee)
                {
                    flaws.push(
                        // type-raw-pointer-pointee-mismatch
                        Diagnostic::new(
                            AddressCode::RawPointerPointeeMismatch,
                            "address-taking pointee does not match the expected raw-pointer type",
                        )
                        .with_arg("address_space", address_space.to_string()),
                    );
                }
            } else if !ty.is_unresolved_address() {
                // type-address-taking-non-pointer
                flaws.push(Diagnostic::new(
                    AddressCode::TakingNonPointer,
                    "address-taking requires a raw-pointer representation",
                ));
            }
        }
        TypedExprKind::Deref(inner) => {
            if typed.exprs.ty.get(inner.as_usize()).is_none_or(|inner_ty| {
                reference_pointee_for_type(inner_ty, Some(&typed.type_registry)).is_none()
                    && typed
                        .type_registry
                        .resolved_definition_for_type(inner_ty)
                        .pointee_ty()
                        .is_none()
            }) {
                // type-deref-non-address
                flaws.push(Diagnostic::new(
                    AddressCode::DereferenceNonAddress,
                    "cannot dereference a value without raw address representation",
                ));
            }
        }
        TypedExprKind::Destructure {
            tag_name,
            value,
            field_bindings,
            ..
        } => {
            let base_ty = typed.exprs.ty.get(value.as_usize()).map(|ty| {
                reference_pointee_for_type(ty, Some(&typed.type_registry))
                    .unwrap_or_else(|| ty.clone())
            });
            let base_definition = base_ty
                .as_ref()
                .map(|ty| typed.type_registry.resolved_definition_for_type(ty));
            let fields = match &base_definition {
                Some(Ty::Record { fields, .. }) => Some(fields.as_slice()),
                Some(Ty::Union { variants, .. }) => {
                    match variants.iter().find(|variant| variant.name == *tag_name) {
                        Some(variant) => Some(variant.fields.as_slice()),
                        None => {
                            flaws.push(
                                // type-unknown-variant
                                Diagnostic::new(
                                    UnionCode::VariantUnknown,
                                    format!("union has no variant `{}`", tag_name.as_str()),
                                )
                                .with_arg("name", tag_name.as_str().to_string()),
                            );
                            None
                        }
                    }
                }
                _ => None,
            };
            if let Some(fields) = fields {
                for (field_name, _) in field_bindings {
                    if !fields.iter().any(|(name, _)| name == field_name) {
                        flaws.push(
                            // type-unknown-field
                            Diagnostic::new(
                                MemberCode::Unknown,
                                format!("variant has no field `{}`", field_name.as_str()),
                            )
                            .with_arg("name", field_name.as_str().to_string()),
                        );
                    }
                }
            }
        }
        TypedExprKind::RecordSet { base, field, .. } => {
            let field_exists = typed.exprs.ty.get(base.as_usize()).and_then(|ty| {
                match typed.type_registry.resolved_definition_for_type(ty) {
                    Ty::Record { fields, .. } => Some(fields.iter().any(|(name, _)| name == field)),
                    _ => None,
                }
            });
            let field_exists = field_exists.or_else(|| {
                let TypedExprKind::FnCall { target, args, .. } =
                    typed.exprs.kind.get(base.as_usize())?
                else {
                    return None;
                };
                if args.as_ref().is_some_and(|args| !args.is_empty()) {
                    return None;
                }
                let bind = typed.defs.get(target)?;
                let body = match &bind.body {
                    BindBody::Expr(expr) => Some(*expr),
                    BindBody::Body { exprs, ret } => (*ret).or_else(|| exprs.last().copied()),
                    BindBody::Unlowered => None,
                }?;
                let TypedExprKind::TagCall { variant_id, .. } =
                    typed.exprs.kind.get(body.as_usize())?
                else {
                    return None;
                };
                let tag = typed.tags.get(&variant_id.union)?;
                Some(tag.record_field_types.contains_key(field))
            });
            if field_exists == Some(false) {
                flaws.push(
                    // type-unknown-field
                    Diagnostic::new(
                        MemberCode::Unknown,
                        format!("record has no field `{}`", field.as_str()),
                    )
                    .with_arg("name", field.as_str().to_string()),
                );
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "../../tests/support/positional_patterns.rs"]
mod positional_pattern_tests;

#[cfg(test)]
#[path = "lower_exprs_constant_tests.rs"]
mod constant_tests;

pub(super) fn slice_type(typed: &TypedFileAst, source: ExprId, length: ExprId) -> Ty {
    Ty::Array {
        elem: Box::new(super::lower_ty::extract_element_type(
            &typed.exprs.ty[source.index()],
            Some(&typed.type_registry),
        )),
        size: super::symbolic_values::normal_expr(typed, None, &HashMap::new(), length)
            .unwrap_or_else(|| super::symbolic_values::call_result_symbol(length)),
    }
}
