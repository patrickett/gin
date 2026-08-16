//! Stage 2: Resolve — Lower parse expressions to the typed arena, attach type flaws.
//!
//! This stage walks the parse-tree expressions, infers/resolves their types,
//! converts them to [`TypedExprKind`], pushes them into the expression arena,
//! and attaches type-check flaws.

use internment::Intern;
use std::collections::{HashMap, HashSet};

use crate::analysis::{TyOwnershipExt, when_is_exhaustive};
use crate::prepare_target::const_env_from_prepared_ast;
use ast::HashFloat;
use ast::prelude::*;
use ast_format::type_expr::ExprFormatExt;

use crate::Normalize;
use crate::intrinsic::IntrinsicOp;
use crate::operator::OperatorRegistry;
use crate::ty::{IntegerInterpretation, Ty, reference_pointee_for_type};
use crate::typed::{
    Availability, BindBody, DefId, ExprId, ReferenceTargetGroup, ReferenceTargetSet, TagId,
    TargetIndex, TypedCallableSignature, TypedExprKind, TypedFileAst, TypedWhenArm, VariantMap,
};
use ast::ConstValue;
use diagnostic::Diagnostic;

use super::TransformCtx;
use super::bounds_check::check_fn_call_bounds;
use super::lower_kind::lower_expr_kind;
use super::lower_tag::is_record_tag_call;
use super::lower_ty::{
    annotate_literal_union_literal, bind_explicit_ty_in_scope, materialize_raw_pointer,
    resolve_expr_type,
};
use super::lower_util::closest_name;

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

/// Local types plus names introduced with `:=` (immutable).
#[derive(Clone, Default)]
pub(crate) struct LocalEnv {
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
    pub(crate) local_callable_returns: HashMap<Intern<String>, Ty>,
    pub(crate) result_evidence: HashMap<Intern<String>, crate::typed::ResultEvidence>,
    pub(crate) projected_result_evidence:
        HashMap<Intern<String>, crate::typed::ProjectedResultEvidence>,
}

pub(crate) fn join_place_versions(
    typed: &mut TypedFileAst,
    env: &mut LocalEnv,
    incoming: &LocalEnv,
    branches: &[LocalEnv],
    include_incoming: bool,
) {
    for (name, place) in &incoming.places {
        let Some(incoming_version) = incoming.place_versions.get(name).copied() else {
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
            env.place_versions.insert(*name, predecessors[0]);
            continue;
        }
        let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
        typed.place_versions.push(crate::typed::TypedPlaceVersion {
            place: *place,
            origin: crate::typed::PlaceVersionOrigin::Join(predecessors),
            ty: typed.places[place.0 as usize].ty.clone(),
            integer_knowledge: None,
        });
        env.place_versions.insert(*name, version);
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
    for (parent, projection) in projected_keys {
        let Some(place) = incoming
            .projected_places
            .get(&(parent, projection.clone()))
            .or_else(|| {
                branches
                    .iter()
                    .find_map(|branch| branch.projected_places.get(&(parent, projection.clone())))
            })
            .copied()
        else {
            continue;
        };
        let incoming_version = if let Some(version) = incoming.projected_versions.get(&place) {
            *version
        } else {
            let Some(base) = current_version_for_place(typed, incoming, parent) else {
                continue;
            };
            let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
            typed.place_versions.push(crate::typed::TypedPlaceVersion {
                place,
                origin: crate::typed::PlaceVersionOrigin::Projection { base },
                ty: typed.places[place.0 as usize].ty.clone(),
                integer_knowledge: None,
            });
            version
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
            let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
            typed.place_versions.push(crate::typed::TypedPlaceVersion {
                place,
                origin: crate::typed::PlaceVersionOrigin::Join(predecessors),
                ty: typed.places[place.0 as usize].ty.clone(),
                integer_knowledge: None,
            });
            version
        };
        env.projected_places.insert((parent, projection), place);
        env.projected_versions.insert(place, version);
    }
}

pub(crate) fn join_loop_place_versions(
    typed: &mut TypedFileAst,
    env: &mut LocalEnv,
    incoming: &LocalEnv,
    body: &LocalEnv,
) {
    for (name, place) in &incoming.places {
        let Some(incoming_version) = incoming.place_versions.get(name).copied() else {
            continue;
        };
        let backedge = if body.places.get(name) == Some(place) {
            body.place_versions.get(name).copied()
        } else {
            None
        };
        let Some(backedge) = backedge.filter(|version| *version != incoming_version) else {
            env.place_versions.insert(*name, incoming_version);
            continue;
        };
        let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
        typed.place_versions.push(crate::typed::TypedPlaceVersion {
            place: *place,
            origin: crate::typed::PlaceVersionOrigin::LoopPhi {
                incoming: incoming_version,
                backedges: vec![backedge],
            },
            ty: typed.places[place.0 as usize].ty.clone(),
            integer_knowledge: None,
        });
        close_loop_version_cycle(typed, backedge, incoming_version, version, *place);
        env.place_versions.insert(*name, version);
    }
    for ((parent, projection), place) in &body.projected_places {
        let incoming_version = if let Some(version) = incoming.projected_versions.get(place) {
            *version
        } else {
            let Some(base) = current_version_for_place(typed, incoming, *parent) else {
                continue;
            };
            let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
            typed.place_versions.push(crate::typed::TypedPlaceVersion {
                place: *place,
                origin: crate::typed::PlaceVersionOrigin::Projection { base },
                ty: typed.places[place.0 as usize].ty.clone(),
                integer_knowledge: None,
            });
            version
        };
        let Some(backedge) = body
            .projected_versions
            .get(place)
            .copied()
            .filter(|version| *version != incoming_version)
        else {
            env.projected_places
                .insert((*parent, projection.clone()), *place);
            env.projected_versions.insert(*place, incoming_version);
            continue;
        };
        let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
        typed.place_versions.push(crate::typed::TypedPlaceVersion {
            place: *place,
            origin: crate::typed::PlaceVersionOrigin::LoopPhi {
                incoming: incoming_version,
                backedges: vec![backedge],
            },
            ty: typed.places[place.0 as usize].ty.clone(),
            integer_knowledge: None,
        });
        close_loop_version_cycle(typed, backedge, incoming_version, version, *place);
        env.projected_places
            .insert((*parent, projection.clone()), *place);
        env.projected_versions.insert(*place, version);
    }
}

fn close_loop_version_cycle(
    typed: &mut TypedFileAst,
    backedge: crate::typed::PlaceVersionId,
    incoming: crate::typed::PlaceVersionId,
    phi: crate::typed::PlaceVersionId,
    place: crate::typed::PlaceId,
) {
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
            | crate::typed::PlaceVersionOrigin::Initializer(_) => {}
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
    pub(crate) locals: &'a HashSet<Intern<String>>,
    /// Current definition being lowered (for SelfRef resolution).
    pub(crate) current_def_id: Option<DefId>,
    pub(crate) callable_signatures: &'a HashMap<DefId, TypedCallableSignature>,
    pub(crate) operator_registry: &'a OperatorRegistry,
    pub(crate) tag_params: &'a HashMap<Intern<String>, Parameters>,
    pub(crate) tag_decls: &'a ast::TagMap,
    pub(crate) integer_literal_defaults: &'a [Ty],
    pub(crate) raw_pointer_defaults: &'a [Ty],
    pub(crate) fixed_array_defaults: &'a [Ty],
    pub(crate) allow_literal_default: bool,
    pub(crate) temporary_shadowing: bool,
}

impl ExprLowerScope<'_> {
    /// Call args, binary operands, and nested `when` subjects do not inherit the outer expected return type.
    pub(crate) fn child(&self) -> Self {
        Self {
            tag_types: self.tag_types,
            variant_map: self.variant_map,
            receiver_type: self.receiver_type,
            expected_ty: None,
            locals: self.locals,
            current_def_id: self.current_def_id,
            callable_signatures: self.callable_signatures,
            operator_registry: self.operator_registry,
            tag_params: self.tag_params,
            tag_decls: self.tag_decls,
            integer_literal_defaults: self.integer_literal_defaults,
            raw_pointer_defaults: self.raw_pointer_defaults,
            fixed_array_defaults: self.fixed_array_defaults,
            allow_literal_default: true,
            temporary_shadowing: self.temporary_shadowing,
        }
    }

    pub(crate) fn with_expected<'a>(&'a self, expected_ty: Option<&'a Ty>) -> ExprLowerScope<'a> {
        ExprLowerScope {
            tag_types: self.tag_types,
            variant_map: self.variant_map,
            receiver_type: self.receiver_type,
            expected_ty,
            locals: self.locals,
            current_def_id: self.current_def_id,
            callable_signatures: self.callable_signatures,
            operator_registry: self.operator_registry,
            tag_params: self.tag_params,
            tag_decls: self.tag_decls,
            integer_literal_defaults: self.integer_literal_defaults,
            raw_pointer_defaults: self.raw_pointer_defaults,
            fixed_array_defaults: self.fixed_array_defaults,
            allow_literal_default: true,
            temporary_shadowing: self.temporary_shadowing,
        }
    }

    pub(crate) fn temporary_child(&self) -> Self {
        let mut child = self.child();
        child.temporary_shadowing = true;
        child
    }

    pub(crate) fn without_literal_default(&self) -> Self {
        Self {
            allow_literal_default: false,
            ..self.child()
        }
    }

    pub(crate) fn with_anonymous_literal_default(&self) -> Self {
        Self {
            integer_literal_defaults: &[],
            allow_literal_default: true,
            temporary_shadowing: false,
            ..self.child()
        }
    }
}

/// Walks all bind bodies and top-level expressions in the `FileAst`,
/// converts parse-tree `Expr` nodes to `TypedExprKind`, pushes them into
/// the expression arena, and attaches type-check flaws.
pub fn stage_lower(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    // Own tag types plus cross-file.
    let mut tag_types: HashMap<Intern<String>, Ty> = ctx
        .cross_file_tag_types
        .iter()
        .map(|(tid, ty)| (tid.0, ty.clone()))
        .collect();
    for (tid, ty) in &typed.tag_types {
        tag_types.insert(tid.0, ty.clone());
    }
    let mut integer_literal_defaults = ctx.integer_literal_defaults.clone();
    integer_literal_defaults.extend(
        typed
            .tags
            .values()
            .filter(|tag| tag.attributes.default_literal == Some(ast::ty::LiteralKind::Integer))
            .map(|tag| tag.resolved_ty.clone()),
    );
    integer_literal_defaults.retain(|ty| {
        crate::representation::Repr::derive(ty, Some(&typed.type_registry)).is_ok()
            && typed.type_registry.integer_validity_for_type(ty).is_some()
    });
    integer_literal_defaults.dedup();
    let mut raw_pointer_defaults = ctx.raw_pointer_defaults.clone();
    raw_pointer_defaults.extend(
        typed
            .tags
            .values()
            .filter(|tag| {
                tag.attributes.default_type_former == Some(ast::DefaultTypeFormerRole::RawPointer)
            })
            .map(|tag| tag.resolved_ty.clone()),
    );
    raw_pointer_defaults.retain(|ty| {
        typed
            .type_registry
            .resolved_definition_for_type(ty)
            .address_space()
            .is_some()
    });
    raw_pointer_defaults.dedup();
    let mut fixed_array_defaults = ctx.fixed_array_defaults.clone();
    fixed_array_defaults.extend(
        typed
            .tags
            .values()
            .filter(|tag| {
                tag.attributes.default_type_former == Some(ast::DefaultTypeFormerRole::FixedArray)
            })
            .map(|tag| tag.resolved_ty.clone()),
    );
    fixed_array_defaults.retain(|ty| ty.type_id().is_some());
    fixed_array_defaults.dedup();

    // Collect variant_map reference before any mutable borrows (own + cross-file).
    let mut variant_map: VariantMap = typed.variant_map.clone();
    for (variant_name, entries) in &ctx.cross_file_variant_map {
        variant_map
            .entry(*variant_name)
            .or_default()
            .extend(entries.iter().cloned());
    }

    // First pass: lower all expressions, collecting body assignments.
    struct DefBodyAssign {
        def_id: DefId,
        body: BindBody,
    }

    let mut def_ids: Vec<DefId> = typed.defs.keys().copied().collect();
    def_ids.sort_by_key(|def_id| {
        let bind = file_ast.defs.get(&def_id.0);
        let is_value_expr = bind
            .is_some_and(|bind| matches!(bind.value, BindValue::Expr(_)) && bind.params.is_none());
        let source_start = bind
            .map(|bind| typed.span_table.get(bind.name_span).start())
            .unwrap_or(usize::MAX);
        (!is_value_expr, source_start, def_id.0.as_str().to_string())
    });

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
                .get(&def_id.0)
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
    let operator_registry = OperatorRegistry::new(
        typed
            .defs
            .iter()
            .map(|(id, bind)| (*id, TypedCallableSignature::from(bind))),
        ctx.cross_file_callable_signatures
            .iter()
            .map(|(id, signature)| (*id, signature.clone())),
    );

    for def_id in &def_ids {
        let Some(bind) = file_ast.defs.get(&def_id.0) else {
            continue;
        };
        let receiver_type = receiver_types.get(def_id).and_then(|r| r.as_ref());
        let empty_locals = HashSet::new();
        let locals: &HashSet<Intern<String>> = param_sets.get(def_id).unwrap_or(&empty_locals);

        let mut env = LocalEnv::default();
        if let Some(typed_bind) = typed.defs.get(def_id) {
            let params = typed_bind.params.clone();
            let groups = typed_bind.param_groups.clone();
            let conventions = typed_bind.param_conventions.clone();
            for (slot, ((name, ty), group)) in params.iter().zip(&groups).enumerate() {
                env.types.insert(*name, ty.clone());
                let place = crate::typed::PlaceId(typed.places.len() as u32);
                typed.places.push(crate::typed::TypedPlace {
                    binder: ast::BinderId::new(
                        typed.file_id.0,
                        ast::BinderOwner::Parameter {
                            definition: def_id.0,
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
                env.target_groups
                    .insert(*name, ReferenceTargetSet::singleton(target));
                let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
                typed.place_versions.push(crate::typed::TypedPlaceVersion {
                    place,
                    origin: crate::typed::PlaceVersionOrigin::Parameter,
                    ty: ty.clone(),
                    integer_knowledge: None,
                });
                env.place_versions.insert(*name, version);
            }
        }

        let mut return_ty =
            bind_explicit_ty_in_scope(bind, &tag_types, &tag_params, &file_ast.tags);
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
            || bind.return_type_name.is_some()
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
                    ret.value
                        .as_ref()
                        .is_some_and(|ret_expr| {
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
            let has_fixed_array_default = fixed_array_defaults.len() == 1
                && file_ast.tags.values().any(|declare| {
                    declare.attributes.default_type_former
                        == Some(ast::DefaultTypeFormerRole::FixedArray)
                        && declare.attributes.fixed_array.is_some()
                });
            let inferred = match &bind.value {
                BindValue::Expr(typed_expr) => Some(resolve_expr_type(
                    typed_expr.as_ref(),
                    &tag_types,
                    &variant_map,
                    receiver_type,
                    None,
                    &env.types,
                    &typed.fn_return_types,
                    &typed.type_registry,
                )),
                BindValue::Body { ret, .. } => Some(
                    ret.value
                        .as_ref()
                        .map(|typed_expr| {
                            resolve_expr_type(
                                typed_expr.as_ref(),
                                &tag_types,
                                &variant_map,
                                receiver_type,
                                None,
                                &env.types,
                                &typed.fn_return_types,
                                &typed.type_registry,
                            )
                        })
                        .unwrap_or(Ty::Unit),
                ),
                _ => None,
            };
            if let Some(ty) = inferred {
                return_ty = Some(
                    if has_fixed_array_default && matches!(ty, Ty::Array { .. }) {
                        force_expected_return_ty = true;
                        Ty::Opaque(Intern::from_ref("unresolved-array-construction"))
                    } else {
                        ty
                    },
                );
            }
        }
        let return_ty = return_ty;
        let scope = ExprLowerScope {
            tag_types: &tag_types,
            variant_map: &variant_map,
            receiver_type,
            expected_ty: force_expected_return_ty
                .then_some(return_ty.as_ref())
                .flatten(),
            locals,
            current_def_id: Some(*def_id),
            callable_signatures: &ctx.cross_file_callable_signatures,
            operator_registry: &operator_registry,
            tag_params: &tag_params,
            tag_decls: &file_ast.tags,
            integer_literal_defaults: &integer_literal_defaults,
            raw_pointer_defaults: &raw_pointer_defaults,
            fixed_array_defaults: &fixed_array_defaults,
            allow_literal_default: true,
            temporary_shadowing: false,
        };

        let body = match &bind.value {
            BindValue::Expr(typed_expr) => {
                let id = lower_typed_expr(typed, typed_expr.as_ref(), &scope, &mut env);
                if let (true, Some(explicit)) = (force_expected_return_ty, return_ty.clone())
                    && !matches!(explicit, Ty::UnresolvedLiteral(_))
                    && !matches!(
                        explicit,
                        Ty::Opaque(name) if name.as_str() == "unresolved-array-construction"
                    )
                {
                    if let (
                        Some(Ty::Named {
                            instance: actual, ..
                        }),
                        Ty::Named {
                            instance: expected, ..
                        },
                    ) = (typed.exprs.ty.get(id.as_usize()), &explicit)
                        && (actual.declaration != expected.declaration
                            || (!named_instance_has_symbolic_argument(actual)
                                && actual.arguments != expected.arguments))
                        && let Some(typed_bind) = typed.defs.get_mut(def_id)
                    {
                        typed_bind.flaws.push(
                            Diagnostic::new(
                                "type-return-mismatch",
                                "returned nominal type does not match the declared return type",
                            )
                            .at_span_id(bind.name_span, &typed.span_table),
                        );
                    }
                    typed.exprs.ty[id.as_usize()] = explicit.clone();
                    let literal_values = typed
                        .type_registry
                        .resolved_definition_for_type(&explicit)
                        .union_literal_values()
                        .map(<[ast::ConstValue]>::to_vec);
                    if let Some(values) = literal_values {
                        annotate_literal_union_literal(typed, id, &explicit, &values);
                    } else if !matches!(
                        typed.exprs.const_value[id.as_usize()],
                        Some(ast::ConstValue::Int(_) | ast::ConstValue::ResultAlternative { .. })
                    ) {
                        typed.exprs.const_value[id.as_usize()] = None;
                    }
                }
                BindBody::Expr(id)
            }
            BindValue::Body { exprs, ret } => {
                let mut lowered_exprs: Vec<ExprId> = Vec::new();
                for expr in exprs {
                    let nested_scope = scope.child();
                    let expr_scope = if matches!(expr.value, Expr::If(_) | Expr::When(_)) {
                        &scope
                    } else {
                        &nested_scope
                    };
                    lowered_exprs.push(lower_typed_expr(typed, expr, expr_scope, &mut env));
                }
                let ret_id = ret
                    .value
                    .as_ref()
                    .map(|ret_expr| lower_typed_expr(typed, ret_expr, &scope, &mut env));
                BindBody::Body {
                    exprs: lowered_exprs,
                    ret: ret_id,
                }
            }
            BindValue::Extern | BindValue::Unassigned => BindBody::Extern,
        };
        let return_expr_id = match &body {
            BindBody::Expr(expr)
            | BindBody::Body {
                ret: Some(expr), ..
            } => Some(*expr),
            BindBody::Body { ret: None, .. } => None,
            BindBody::Extern => None,
        };
        let inferred_return_type = if bind.return_tag.is_none()
            && bind.return_type_name.is_none()
            && bind.anonymous_result_alternatives.is_empty()
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
                BindBody::Extern => None,
            }
        } else {
            None
        };
        let result_exit_owners = result_family_exit_owners(typed, &body);
        if result_exit_owners.len() > 1
            && let Some(typed_bind) = typed.defs.get_mut(def_id)
        {
            typed_bind.flaws.push(
                Diagnostic::new(
                    "type-result-family-mixed-owners",
                    "an inferred callable cannot combine anonymous result families with different owners",
                )
                .at_span_id(bind.name_span, &typed.span_table),
            );
        }
        if result_exit_owners
            .iter()
            .any(|owner| matches!(owner, ast::ResultFamilyOwner::LocalCallable(_)))
            && let Some(typed_bind) = typed.defs.get_mut(def_id)
        {
            typed_bind.flaws.push(
                Diagnostic::new(
                    "type-local-result-family-escapes",
                    "a local callable result family cannot escape its enclosing callable",
                )
                .at_span_id(bind.name_span, &typed.span_table),
            );
        }
        if let (Some(return_expr_id), Some(return_type)) =
            (return_expr_id, inferred_return_type.as_ref())
            && let Some(stored) = typed.exprs.ty.get_mut(return_expr_id.as_usize())
            && stored != return_type
        {
            if let (
                Ty::Named {
                    instance: actual, ..
                },
                Ty::Named {
                    instance: expected, ..
                },
            ) = (&*stored, return_type)
                && (actual.declaration != expected.declaration
                    || (!named_instance_has_symbolic_argument(actual)
                        && actual.arguments != expected.arguments))
                && let Some(typed_bind) = typed.defs.get_mut(def_id)
            {
                typed_bind.flaws.push(
                    Diagnostic::new(
                        "type-return-mismatch",
                        "returned nominal type does not match the declared return type",
                    )
                    .at_span_id(bind.name_span, &typed.span_table),
                );
            }
            *stored = return_type.clone();
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
            if let Some(typed_bind) = typed.defs.get_mut(def_id) {
                typed_bind.return_type = explicit_return_type.clone();
            }
        }
        if let Some(return_type) = inferred_return_type.as_ref() {
            typed.fn_return_types.insert(*def_id, return_type.clone());
        }
        if let Some(typed_bind) = typed.defs.get_mut(def_id) {
            typed_bind.body = body.clone();
            typed_bind.return_evidence = return_expr_id
                .and_then(|expr| typed.exprs.result_evidence_of(expr).cloned().flatten());
            if let Some(return_type) = inferred_return_type {
                typed_bind.return_type = return_type;
            }
        }
        assignments.push(DefBodyAssign {
            def_id: *def_id,
            body,
        });
    }
    refresh_result_evidence_summaries(typed, def_ids.len().saturating_add(1));

    // Lower top-level expressions.
    let empty_locals = HashSet::new();
    let mut env = LocalEnv::default();
    let scope = ExprLowerScope {
        tag_types: &tag_types,
        variant_map: &variant_map,
        receiver_type: None,
        expected_ty: None,
        locals: &empty_locals,
        current_def_id: None,
        callable_signatures: &ctx.cross_file_callable_signatures,
        operator_registry: &operator_registry,
        tag_params: &tag_params,
        tag_decls: &file_ast.tags,
        integer_literal_defaults: &integer_literal_defaults,
        raw_pointer_defaults: &raw_pointer_defaults,
        fixed_array_defaults: &fixed_array_defaults,
        allow_literal_default: true,
        temporary_shadowing: false,
    };
    for (expr, span_id) in &file_ast.exprs {
        let wrapped = Typed::infer(expr.clone(), *span_id);
        let expr_id = lower_typed_expr(typed, &wrapped, &scope, &mut env);
        typed.root_exprs.push(expr_id);
    }

    // Check for redundant `self` parameter type annotations on methods.
    for def_id in &def_ids {
        let Some(bind) = file_ast.defs.get(&def_id.0) else {
            continue;
        };
        if !bind.is_method() {
            continue;
        }
        if let Some(params) = &bind.params
            && params.iter().any(|(name, kind)| {
                name.as_str() == "self" && matches!(&kind.kind, ParameterKind::Tagged(_))
            })
            && let Some(typed_bind) = typed.defs.get_mut(def_id)
        {
            typed_bind.flaws.push(Diagnostic::new(
                "type-self-param-typed",
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
}

fn result_family_exit_owners(
    typed: &TypedFileAst,
    body: &BindBody,
) -> HashSet<ast::ResultFamilyOwner> {
    let mut owners = HashSet::new();
    match body {
        BindBody::Expr(exit) => collect_result_family_exit_owners(typed, *exit, &mut owners),
        BindBody::Body { exprs, ret } => {
            for expr in exprs {
                if matches!(
                    typed.exprs.kind[expr.as_usize()],
                    TypedExprKind::If(_) | TypedExprKind::When(_)
                ) {
                    collect_result_family_exit_owners(typed, *expr, &mut owners);
                }
            }
            if let Some(exit) = ret {
                collect_result_family_exit_owners(typed, *exit, &mut owners);
            }
        }
        BindBody::Extern => {}
    }
    owners
}

fn collect_result_family_exit_owners(
    typed: &TypedFileAst,
    expr: ExprId,
    owners: &mut HashSet<ast::ResultFamilyOwner>,
) {
    match &typed.exprs.kind[expr.as_usize()] {
        TypedExprKind::If(if_expr) => {
            if let Some(exit) = if_expr.stmts.last() {
                collect_result_family_exit_owners(typed, *exit, owners);
            }
            if let Some(exit) = if_expr.ret {
                collect_result_family_exit_owners(typed, exit, owners);
            }
        }
        TypedExprKind::When(when_expr) => {
            for arm in &when_expr.arms {
                let exit = match arm {
                    TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. } => *body,
                    TypedWhenArm::Else(body, _) => *body,
                };
                collect_result_family_exit_owners(typed, exit, owners);
            }
        }
        TypedExprKind::FnCall { target, .. } => {
            if let Some(Ty::ResultFamily { owner, .. }) = typed
                .defs
                .get(target)
                .map(|bind| &bind.return_type)
                .or_else(|| typed.fn_return_types.get(target))
            {
                owners.insert(owner.clone());
            } else if let Ty::ResultFamily { owner, .. } = &typed.exprs.ty[expr.as_usize()] {
                owners.insert(owner.clone());
            }
        }
        _ => {
            if let Ty::ResultFamily { owner, .. } = &typed.exprs.ty[expr.as_usize()] {
                owners.insert(owner.clone());
            }
        }
    }
}

fn refresh_result_evidence_summaries(typed: &mut TypedFileAst, max_passes: usize) {
    for _ in 0..max_passes {
        let mut changed = false;
        for index in 0..typed.exprs.kind.len() {
            let kind = typed.exprs.kind[index].clone();
            let ty = typed.exprs.ty[index].clone();
            let evidence = match &kind {
                TypedExprKind::Bind { body, .. } => {
                    typed.exprs.result_evidence_of(*body).cloned().flatten()
                }
                _ => result_evidence_for(&kind, &ty, typed),
            };
            if evidence.is_some() && typed.exprs.result_evidence[index] != evidence {
                typed.exprs.result_evidence[index] = evidence;
                changed = true;
            }
        }
        for bind in typed.defs.values_mut() {
            let returned = match &bind.body {
                BindBody::Expr(expr)
                | BindBody::Body {
                    ret: Some(expr), ..
                } => typed.exprs.result_evidence_of(*expr).cloned().flatten(),
                BindBody::Body { ret: None, .. } | BindBody::Extern => None,
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
    let former = declaration.fixed_array.as_ref()?;

    let mut instance = instance.clone();
    let mut saw_length = false;
    let mut saw_element = false;
    for (parameter, argument) in &mut instance.arguments {
        if *parameter == former.length_parameter {
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
    env: &mut LocalEnv,
) -> ExprId {
    let shadowed_place = match &expr.value {
        Expr::Bind(bind) if !bind.is_rebind() && !scope.temporary_shadowing => {
            env.places.get(&bind.name).copied()
        }
        _ => None,
    };
    let mut resolved_ty = resolve_expr_type(
        expr,
        scope.tag_types,
        scope.variant_map,
        scope.receiver_type,
        scope.expected_ty,
        &env.types,
        &typed.fn_return_types,
        &typed.type_registry,
    );
    // Infer const_value from literals and known value refs at parse time (before const-folding runs).
    let mut const_val = expr.const_value.clone().or_else(|| match &expr.value {
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
                const_for_def_id(target, typed).or_else(|| {
                    scope.variant_map.get(&target.0).and_then(|entries| {
                        if entries.len() == 1 {
                            let qual_path = (!call.path.value.segments.is_empty()).then(|| {
                                let mut path = call.path.value.root.as_str().to_string();
                                for segment in call
                                    .path
                                    .value
                                    .segments
                                    .iter()
                                    .take(call.path.value.segments.len().saturating_sub(1))
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
        Expr::Lit(lit) => Some(match lit {
            ast::Literal::Number(n) => ast::ConstValue::Int((*n as u128).into()),
            ast::Literal::Int(n) => ast::ConstValue::Int(*n),
            ast::Literal::Float(HashFloat(f)) => ast::ConstValue::Float(HashFloat(*f)),
            ast::Literal::String(s) => ast::ConstValue::String(s.clone()),
        }),
        Expr::Bind(bind) => match &bind.value {
            // For `x := 1`, look at the inner literal directly.
            ast::BindValue::Expr(inner) => match &inner.value {
                Expr::Lit(lit) => Some(match lit {
                    ast::Literal::Number(n) => ast::ConstValue::Int((*n as u128).into()),
                    ast::Literal::Int(n) => ast::ConstValue::Int(*n),
                    ast::Literal::Float(HashFloat(f)) => ast::ConstValue::Float(HashFloat(*f)),
                    ast::Literal::String(s) => ast::ConstValue::String(s.clone()),
                }),
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
        && matches!(inner.value, Expr::Lit(Literal::Int(_) | Literal::Number(_)))
        && let Some(ast::ConstValue::Int(value)) = inner.const_value.clone().or({
            match &inner.value {
                Expr::Lit(Literal::Int(value)) => Some(ast::ConstValue::Int(*value)),
                Expr::Lit(Literal::Number(value)) => {
                    Some(ast::ConstValue::Int((*value as u128).into()))
                }
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

    let reserved_expr_id = matches!(expr.value, Expr::FnCall(_) | Expr::Bind(_)).then(|| {
        let expr_id = ExprId(typed.exprs.kind.len() as u32);
        typed
            .exprs
            .kind
            .push(TypedExprKind::Lit(Literal::Number(0)));
        typed.exprs.ty.push(Ty::Unit);
        typed.exprs.span.push(expr.span_id);
        typed.exprs.const_value.push(None);
        typed.exprs.integer_knowledge.push(None);
        typed.exprs.result_evidence.push(None);
        typed.exprs.projected_result_evidence.push(HashMap::new());
        typed.exprs.availability.push(Availability::Unknown);
        typed.exprs.target_group.push(None);
        typed.exprs.place.push(None);
        typed.exprs.place_version.push(None);
        typed.exprs.flaws.push(Vec::new());
        expr_id
    });
    let kind = lower_expr_kind(typed, expr, scope, env, reserved_expr_id);

    if let TypedExprKind::IntrinsicCall { op, args } = &kind
        && matches!(
            op,
            IntrinsicOp::BitsAdd | IntrinsicOp::BitsSubtract | IntrinsicOp::BitsMultiply
        )
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

    if let TypedExprKind::IntrinsicCall {
        op: IntrinsicOp::BitsCompare { predicate, .. },
        ..
    } = &kind
    {
        resolved_ty = structural_comparison_result_family(*predicate);
    }

    if matches!(
        resolved_ty,
        Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
    ) && let TypedExprKind::IntrinsicCall { op, args } = &kind
        && args.len() == 2
    {
        let lhs = &typed.exprs.ty[args[0].as_usize()];
        let rhs = &typed.exprs.ty[args[1].as_usize()];
        if matches!(op, IntrinsicOp::BitsCompare { .. }) {
            let IntrinsicOp::BitsCompare { predicate, .. } = op else {
                unreachable!()
            };
            resolved_ty = structural_comparison_result_family(*predicate);
        } else {
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
    }

    // For compile-time resolved when expressions, propagate the body's const_value.
    let mut const_val = if let TypedExprKind::When(ref when_expr) = kind {
        if when_expr.arms.len() == 1 && when_expr.subject.is_some() {
            let body_id = match &when_expr.arms[0] {
                TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. } => *body,
                TypedWhenArm::Else(body, _) => *body,
            };
            typed.exprs.const_value[body_id.as_usize()]
                .clone()
                .or(const_val)
        } else {
            const_val
        }
    } else {
        const_val
    };
    if const_val.is_none()
        && let TypedExprKind::IntrinsicCall { op, args } = &kind
        && matches!(op, IntrinsicOp::BitsCompare { .. })
    {
        let values = args
            .iter()
            .map(|argument| typed.exprs.const_value[argument.as_usize()].clone())
            .collect::<Option<Vec<_>>>();
        let width = args
            .iter()
            .filter_map(|argument| typed.exprs.ty[argument.as_usize()].anonymous_integer_width())
            .max()
            .unwrap_or(64);
        if let Some(values) = values {
            const_val = op.fold(&values, u32::from(width), 1).ok().flatten();
        }
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
            ConstValue::Int(value)
                if matches!(
                    kind,
                    TypedExprKind::IntrinsicCall {
                        op: IntrinsicOp::BitsCompare { .. },
                        ..
                    }
                ) =>
            {
                Some(if *value != i256::I256::from(0) {
                    Intern::from_ref("True")
                } else {
                    Intern::from_ref("False")
                })
            }
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
    if let Some(place) = shadowed_place
        && let Some(old_place) = typed.places.get(place.0 as usize)
        && !old_place.ty.is_structurally_discardable()
        && !typed_kind_consumes_place(typed, &kind, place)
    {
        flaws.push(
            Diagnostic::new(
                "type-shadow-would-hide-live-value",
                format!(
                    "fresh binding `{}` would permanently hide a live owned value",
                    old_place.name.as_str()
                ),
            )
            .with_arg("name", old_place.name.as_str().to_string())
            .at_span_id(expr.span_id, &typed.span_table),
        );
    }
    if let TypedExprKind::TakePtr(inner) = &kind
        && resolved_ty.is_unresolved_address()
    {
        let pointee = typed.exprs.ty[inner.as_usize()].clone();
        match scope.raw_pointer_defaults {
            [default] => {
                resolved_ty = materialize_raw_pointer(default, &pointee, &typed.type_registry)
                    .unwrap_or(resolved_ty);
            }
            [] => flaws.push(
                Diagnostic::new(
                    "type-missing-default-raw-pointer",
                    "address-taking needs an expected raw-pointer type or a visible default",
                )
                .at_span_id(expr.span_id, &typed.span_table),
            ),
            _ => flaws.push(
                Diagnostic::new(
                    "type-ambiguous-default-raw-pointer",
                    "address-taking has multiple visible default raw-pointer type formers",
                )
                .at_span_id(expr.span_id, &typed.span_table),
            ),
        }
    }
    if matches!(kind, TypedExprKind::TupleAlloc { .. })
        && matches!(resolved_ty, Ty::Opaque(name) if name.as_str() == "unresolved-array-construction")
    {
        match scope.fixed_array_defaults {
            [default] => {
                let contract = scope
                    .tag_decls
                    .values()
                    .find(|declare| {
                        declare.attributes.default_type_former
                            == Some(ast::DefaultTypeFormerRole::FixedArray)
                    })
                    .and_then(|declare| declare.attributes.fixed_array.as_ref());
                if let Some(contract) = contract {
                    if let TypedExprKind::TupleAlloc { init, size } = &kind {
                        resolved_ty = super::lower_ty::materialize_fixed_array(
                            default,
                            contract,
                            &typed.exprs.ty[init.as_usize()],
                            size.clone(),
                        )
                        .unwrap_or(resolved_ty);
                    }
                } else {
                    flaws.push(Diagnostic::new(
                        "type-invalid-default-array-former",
                        "the visible default fixed-array former has no valid contract",
                    ));
                }
            }
            [] => flaws.push(
                Diagnostic::new(
                    "type-missing-default-fixed-array",
                    "fixed-array construction needs an expected array type or a visible default",
                )
                .at_span_id(expr.span_id, &typed.span_table),
            ),
            _ => flaws.push(
                Diagnostic::new(
                    "type-ambiguous-default-fixed-array",
                    "fixed-array construction has multiple visible default formers",
                )
                .at_span_id(expr.span_id, &typed.span_table),
            ),
        }
    }
    if let TypedExprKind::TupleAlloc { size, .. } = &kind
        && let Ok(crate::representation::Repr::Array { length, .. }) =
            crate::representation::Repr::derive(&resolved_ty, Some(&typed.type_registry))
    {
        let matches_length = size.normalize_to_value().is_some_and(|value| {
                matches!(value, ast::ConstValue::Int(value) if ast::integer::to_u64(value) == Some(length))
            });
        if !matches_length {
            flaws.push(
                Diagnostic::new(
                    "type-fixed-array-length-mismatch",
                    "fixed-array construction length does not match the selected array type",
                )
                .at_span_id(expr.span_id, &typed.span_table),
            );
        }
    }

    let expr_id = reserved_expr_id.unwrap_or(ExprId(typed.exprs.kind.len() as u32));

    if let TypedExprKind::InvalidOperator {
        role,
        lhs,
        rhs,
        error,
    } = &kind
    {
        let lhs_ty = &typed.exprs.ty[lhs.as_usize()];
        let rhs_ty = &typed.exprs.ty[rhs.as_usize()];
        let lhs_is_nominal = lhs_ty
            .named_instance_stripping_reference_wrappers()
            .is_some();
        let rhs_is_nominal = rhs_ty
            .named_instance_stripping_reference_wrappers()
            .is_some();
        let lhs_is_anonymous_int = lhs_ty.is_int() && !lhs_is_nominal;
        let rhs_is_anonymous_int = rhs_ty.is_int() && !rhs_is_nominal;
        let one_nominal = lhs_is_nominal || rhs_is_nominal;
        let is_comparison = matches!(
            role,
            ast::prelude::OperatorRole::Less
                | ast::prelude::OperatorRole::LessOrEqual
                | ast::prelude::OperatorRole::Greater
                | ast::prelude::OperatorRole::GreaterOrEqual
                | ast::prelude::OperatorRole::Equal
        );
        let mixed_anonymous_nominal_comparison = is_comparison
            && ((lhs_is_nominal && rhs_is_anonymous_int)
                || (rhs_is_nominal && lhs_is_anonymous_int));
        let incompatible_result_families = matches!(role, ast::OperatorRole::Equal)
            && matches!(lhs_ty, Ty::ResultFamily { .. })
            && matches!(rhs_ty, Ty::ResultFamily { .. })
            && lhs_ty != rhs_ty;
        let (code, message) = if incompatible_result_families {
            (
                "type-result-family-incompatible",
                format!(
                    "comparison result families `{}` and `{}` have different semantic owners",
                    lhs_ty.format_for_hover(),
                    rhs_ty.format_for_hover()
                ),
            )
        } else if mixed_anonymous_nominal_comparison {
            (
                "type-mixed-anonymous-nominal-comparison",
                format!(
                    "nominal comparison requires one nominal operator for `{lhs_ty:?}` and `{rhs_ty:?}`",
                ),
            )
        } else {
            match error {
                crate::operator::OperatorResolutionError::NoVisibleContract => {
                    if one_nominal {
                        (
                            "type-nominal-operator-unavailable",
                            format!(
                                "nominal operator {role:?} is not provided for `{lhs_ty:?}` and `{rhs_ty:?}`"
                            ),
                        )
                    } else {
                        (
                            "type-no-visible-operator-contract",
                            format!("no visible operator contract is registered for {role:?}"),
                        )
                    }
                }
                crate::operator::OperatorResolutionError::OperandDoesNotProvide => {
                    if one_nominal {
                        (
                            "type-nominal-operator-unavailable",
                            format!(
                                "nominal operator {role:?} is not provided for `{lhs_ty:?}` and `{rhs_ty:?}`",
                            ),
                        )
                    } else {
                        (
                            "type-operator-not-provided",
                            format!(
                                "operator {role:?} is not provided for `{lhs_ty:?}` and `{rhs_ty:?}`"
                            ),
                        )
                    }
                }
                crate::operator::OperatorResolutionError::Ambiguous => (
                    "type-ambiguous-operator",
                    format!("multiple visible {role:?} implementations apply to these operands"),
                ),
            }
        };
        flaws.push(
            Diagnostic::new(code, message)
                .with_arg("role", format!("{role:?}"))
                .with_arg("left_type", format!("{lhs_ty:?}"))
                .with_arg("right_type", format!("{rhs_ty:?}")),
        );
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
                typed.exprs.const_value[args[0].as_usize()].as_ref(),
                typed.exprs.const_value[args[1].as_usize()].as_ref(),
            )
        {
            let binary = match op {
                IntrinsicOp::BitsAdd => Some(ast::BinOp::Add),
                IntrinsicOp::BitsSubtract => Some(ast::BinOp::Subtract),
                IntrinsicOp::BitsMultiply => Some(ast::BinOp::Multiply),
                _ => None,
            };
            if let Some(binary) = binary {
                const_val = lhs.eval_binop(&binary, rhs);
            }
        } else if !anonymous_integer_result && !anonymous_intrinsic_operand_call {
            if let Err(error) =
                op.validate_signature(&operand_types, &resolved_ty, Some(&typed.type_registry))
            {
                flaws.push(op.diagnostic(&error));
            } else {
                let values: Option<Vec<_>> = args
                    .iter()
                    .map(|arg| typed.exprs.const_value[arg.as_usize()].clone())
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
            typed.exprs.const_value[lhs.as_usize()].as_ref(),
            typed.exprs.const_value[rhs.as_usize()].as_ref(),
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
                Diagnostic::new(
                    "type-reassign-constant",
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
                Diagnostic::new(
                    "type-rebind-immutable",
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
        &kind,
        contextual_result_ty.unwrap_or(&resolved_ty),
        typed,
        name_span_id,
        scope.locals,
        &env.types,
        &mut flaws,
    );

    // If the expression has a substituted type (from const-generic param resolution),
    // use it instead of the inferred type.
    let mut final_ty = match &kind {
        TypedExprKind::TagCall { .. } if contextual_result_ty.is_some() => {
            contextual_result_ty.cloned().unwrap()
        }
        TypedExprKind::FnCall {
            substituted_ty: Some(ty),
            ..
        } => ty.clone(),
        TypedExprKind::Bind { name, .. } => env
            .types
            .get(name)
            .cloned()
            .unwrap_or_else(|| resolved_ty.clone()),
        TypedExprKind::When(when_expr) => scope
            .expected_ty
            .cloned()
            .or_else(|| {
                when_expr.arms.iter().find_map(|arm| {
                    let body = match arm {
                        TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. } => *body,
                        TypedWhenArm::Else(body, _) => *body,
                    };
                    typed
                        .exprs
                        .ty
                        .get(body.as_usize())
                        .filter(|ty| !matches!(ty, Ty::UnresolvedLiteral(_)))
                        .cloned()
                })
            })
            .unwrap_or_else(|| resolved_ty.clone()),
        TypedExprKind::If(_) => Ty::Unit,
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
        let has_int_annotation = scope.expected_ty.is_some_and(|ty| {
            matches!(
                typed.type_registry.resolved_definition_for_type(ty),
                Ty::Opaque(name) if name.as_str() == "Int"
            )
        });
        let selected = scope
            .expected_ty
            .and_then(|expected| materialized_integer_expected(expected, &typed.type_registry))
            .or_else(|| {
                if has_int_annotation {
                    return None;
                }
                scope.integer_literal_defaults.first().cloned().filter(|_| {
                    scope.allow_literal_default && scope.integer_literal_defaults.len() == 1
                })
            });
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
        } else if scope.allow_literal_default {
            if scope.expected_ty.is_none_or(|ty| {
                matches!(ty, Ty::UnresolvedLiteral(_))
                    || is_generic_opaque_ty(ty, &typed.type_registry)
            }) {
                let default = scope.integer_literal_defaults.first().cloned();
                if let Some(default) = default {
                    match crate::integer_literal::validate_with_registry(
                        *value,
                        &default,
                        &typed.type_registry,
                    ) {
                        Ok(()) => final_ty = default,
                        Err(error) => {
                            if scope.integer_literal_defaults.len() == 1 {
                                flaws.push(
                                    crate::integer_literal::diagnostic(*value, &default, &error)
                                        .at_span_id(expr.span_id, &typed.span_table),
                                )
                            } else {
                                flaws.push(
                                    Diagnostic::new(
                                        "type-ambiguous-integer-literal-default",
                                        format!(
                                            "integer literal `{value}` has multiple visible defaults"
                                        ),
                                    )
                                    .with_arg("value", value.to_string())
                                    .at_span_id(expr.span_id, &typed.span_table),
                                );
                            }
                        }
                    }
                } else {
                    let width = integer_width_for_exact_value(*value);
                    let signed = *value < i256::I256::from(0);
                    final_ty = Ty::anonymous_integer_for_width(width, signed);
                }
            } else {
                if scope.integer_literal_defaults.len() > 1 {
                    flaws.push(
                        Diagnostic::new(
                            "type-ambiguous-integer-literal-default",
                            format!("integer literal `{value}` has multiple visible defaults"),
                        )
                        .with_arg("value", value.to_string())
                        .at_span_id(expr.span_id, &typed.span_table),
                    );
                }
            }
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
    let target_group = target_group_for_kind(&kind, typed, scope, env, &mut flaws);
    let place = place_for_kind(&kind, &final_ty, typed, env);
    let place_version = place_version_for_kind(&kind, place, typed, env);
    let integer_knowledge =
        integer_knowledge_for(&final_ty, const_val.as_ref(), &typed.type_registry);
    let result_evidence = match &kind {
        TypedExprKind::Bind { body, .. } => {
            typed.exprs.result_evidence_of(*body).cloned().flatten()
        }
        TypedExprKind::FnCall {
            target, args: None, ..
        } if env.locals.contains(&target.0) => env.result_evidence.get(&target.0).cloned(),
        TypedExprKind::TupleGet { base, index } => typed
            .exprs
            .projected_result_evidence_of(*base)
            .and_then(|evidence| evidence.get(&vec![*index]).cloned()),
        _ => result_evidence_for(&kind, &final_ty, typed),
    };
    let projected_result_evidence = projected_result_evidence_for(&kind, typed, env);
    if reserved_expr_id.is_some() {
        let index = expr_id.as_usize();
        typed.exprs.kind[index] = kind;
        typed.exprs.ty[index] = final_ty;
        typed.exprs.span[index] = expr.span_id;
        typed.exprs.const_value[index] = const_val;
        typed.exprs.integer_knowledge[index] = integer_knowledge;
        typed.exprs.result_evidence[index] = result_evidence;
        typed.exprs.projected_result_evidence[index] = projected_result_evidence;
        typed.exprs.availability[index] = Availability::Unknown;
        typed.exprs.target_group[index] = target_group;
        typed.exprs.place[index] = place;
        typed.exprs.place_version[index] = place_version;
        typed.exprs.flaws[index] = flaws;
    } else {
        typed.exprs.kind.push(kind);
        typed.exprs.ty.push(final_ty);
        typed.exprs.span.push(expr.span_id);
        typed.exprs.const_value.push(const_val);
        typed.exprs.integer_knowledge.push(integer_knowledge);
        typed.exprs.result_evidence.push(result_evidence);
        typed
            .exprs
            .projected_result_evidence
            .push(projected_result_evidence);
        typed.exprs.availability.push(Availability::Unknown);
        typed.exprs.target_group.push(target_group);
        typed.exprs.place.push(place);
        typed.exprs.place_version.push(place_version);
        typed.exprs.flaws.push(flaws);
    }

    if expr.span_id.is_valid() {
        let span = typed.span_table.get(expr.span_id);
        typed
            .span_to_expr
            .entry(span.start_u32())
            .or_insert(expr_id);
    }

    expr_id
}

fn place_version_for_kind(
    kind: &TypedExprKind,
    place: Option<crate::typed::PlaceId>,
    typed: &mut TypedFileAst,
    env: &mut LocalEnv,
) -> Option<crate::typed::PlaceVersionId> {
    let projected_write = match kind {
        TypedExprKind::RecordSet { value, .. }
        | TypedExprKind::TupleSet { value, .. }
        | TypedExprKind::BufSet { value, .. } => Some(*value),
        _ => None,
    };
    if let (Some(place), Some(value)) = (place, projected_write) {
        let predecessor = current_version_for_place(typed, env, place);
        let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
        typed.place_versions.push(crate::typed::TypedPlaceVersion {
            place,
            origin: crate::typed::PlaceVersionOrigin::Rebind { value, predecessor },
            ty: typed.places[place.0 as usize].ty.clone(),
            integer_knowledge: typed.exprs.integer_knowledge_of(value).cloned().flatten(),
        });
        env.projected_versions.insert(place, version);
        return Some(version);
    }
    match kind {
        TypedExprKind::FnCall {
            target, args: None, ..
        } => env.place_versions.get(&target.0).copied(),
        TypedExprKind::Bind { name, .. } | TypedExprKind::Reassign { name, .. } => {
            env.place_versions.get(name).copied()
        }
        TypedExprKind::TupleGet { .. } | TypedExprKind::BufGet { .. } | TypedExprKind::Deref(_) => {
            place.and_then(|place| current_version_for_place(typed, env, place))
        }
        _ => None,
    }
}

fn typed_kind_consumes_place(
    typed: &TypedFileAst,
    kind: &TypedExprKind,
    place: crate::typed::PlaceId,
) -> bool {
    let mut consumes = false;
    let _ = crate::typed::walk_expr_children(kind, &mut |child| {
        if matches!(
            typed.exprs.kind_of(child),
            Some(TypedExprKind::Eat(inner) | TypedExprKind::ConsumeArg(inner))
                if typed.exprs.place[inner.as_usize()] == Some(place)
        ) || typed
            .exprs
            .kind_of(child)
            .is_some_and(|child_kind| typed_kind_consumes_place(typed, child_kind, place))
        {
            consumes = true;
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    });
    consumes
}

fn place_for_kind(
    kind: &TypedExprKind,
    ty: &Ty,
    typed: &mut TypedFileAst,
    env: &mut LocalEnv,
) -> Option<crate::typed::PlaceId> {
    match kind {
        TypedExprKind::FnCall {
            target, args: None, ..
        } => env.places.get(&target.0).copied(),
        TypedExprKind::Bind { name, .. } | TypedExprKind::Reassign { name, .. } => {
            env.places.get(name).copied()
        }
        TypedExprKind::TupleGet { base, index } | TypedExprKind::TupleSet { base, index, .. } => {
            let base_definition = typed
                .type_registry
                .resolved_definition_for_type(typed.exprs.ty_of(*base)?);
            let projection_ty = match base_definition {
                Ty::Tuple(fields) => fields.get(*index).cloned().unwrap_or_else(|| ty.clone()),
                Ty::Array { elem, .. } => elem.as_ref().clone(),
                _ => ty.clone(),
            };
            projected_place(
                typed,
                env,
                *base,
                crate::typed::PlaceProjection::Field(*index),
                projection_ty,
            )
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
        TypedExprKind::BufGet { buf, index } | TypedExprKind::BufSet { buf, index, .. } => {
            let projection_ty = typed
                .exprs
                .ty_of(*buf)
                .and_then(Ty::pointee_ty)
                .cloned()
                .unwrap_or_else(|| ty.clone());
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
    env: &mut LocalEnv,
    base: ExprId,
    projection: crate::typed::PlaceProjection,
    ty: Ty,
) -> Option<crate::typed::PlaceId> {
    let parent = typed.exprs.place.get(base.as_usize()).copied().flatten()?;
    let parent_place = typed.places[parent.0 as usize].clone();
    let base_version = typed
        .exprs
        .place_version
        .get(base.as_usize())
        .copied()
        .flatten()?;
    let place = env
        .projected_places
        .get(&(parent, projection.clone()))
        .copied()
        .or_else(|| {
            typed.places.iter().enumerate().find_map(|(index, place)| {
                (place.parent == Some(parent) && place.projection.as_ref() == Some(&projection))
                    .then_some(crate::typed::PlaceId(index as u32))
            })
        })
        .unwrap_or_else(|| {
            let place = crate::typed::PlaceId(typed.places.len() as u32);
            typed.places.push(crate::typed::TypedPlace {
                binder: parent_place.binder,
                name: parent_place.name,
                parent: Some(parent),
                projection: Some(projection.clone()),
                mutable: parent_place.mutable,
                explicit_contract: true,
                ty: ty.clone(),
            });
            place
        });
    env.projected_places.insert((parent, projection), place);
    if let std::collections::hash_map::Entry::Vacant(entry) = env.projected_versions.entry(place) {
        let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
        typed.place_versions.push(crate::typed::TypedPlaceVersion {
            place,
            origin: crate::typed::PlaceVersionOrigin::Projection { base: base_version },
            ty,
            integer_knowledge: None,
        });
        entry.insert(version);
    }
    Some(place)
}

fn current_version_for_place(
    typed: &TypedFileAst,
    env: &LocalEnv,
    place: crate::typed::PlaceId,
) -> Option<crate::typed::PlaceVersionId> {
    env.projected_versions.get(&place).copied().or_else(|| {
        let place = typed.places.get(place.0 as usize)?;
        place
            .parent
            .is_none()
            .then(|| env.place_versions.get(&place.name).copied())
            .flatten()
    })
}

fn projected_result_evidence_for(
    kind: &TypedExprKind,
    typed: &TypedFileAst,
    env: &LocalEnv,
) -> crate::typed::ProjectedResultEvidence {
    match kind {
        TypedExprKind::Bind { body, .. } => typed
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
        TypedExprKind::TupleLit(items) => {
            let mut projected = crate::typed::ProjectedResultEvidence::new();
            for (index, item) in items.iter().enumerate() {
                if let Some(evidence) = typed.exprs.result_evidence_of(*item).cloned().flatten() {
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
        TypedExprKind::TupleGet { base, index } => typed
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

fn structural_comparison_result_family(predicate: crate::intrinsic::ComparisonPredicate) -> Ty {
    let role = match predicate {
        crate::intrinsic::ComparisonPredicate::Less => ast::OperatorRole::Less,
        crate::intrinsic::ComparisonPredicate::LessOrEqual => ast::OperatorRole::LessOrEqual,
        crate::intrinsic::ComparisonPredicate::Greater => ast::OperatorRole::Greater,
        crate::intrinsic::ComparisonPredicate::GreaterOrEqual => ast::OperatorRole::GreaterOrEqual,
        crate::intrinsic::ComparisonPredicate::Equal => ast::OperatorRole::Equal,
    };
    Ty::ResultFamily {
        owner: ast::ResultFamilyOwner::Structural(role),
        alternatives: vec![
            ast::ResultAlternative {
                label: Intern::from_ref("False"),
                proposition: None,
            },
            ast::ResultAlternative {
                label: Intern::from_ref("True"),
                proposition: None,
            },
        ],
    }
}

fn result_evidence_for(
    kind: &TypedExprKind,
    ty: &Ty,
    typed: &TypedFileAst,
) -> Option<crate::typed::ResultEvidence> {
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
            BindBody::Body { ret: None, .. } | BindBody::Extern => None,
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
        && let Some(evidence) = typed.exprs.result_evidence_of(*argument).cloned().flatten()
    {
        return Some(evidence);
    }
    if let TypedExprKind::FnCall {
        target,
        args: Some(args),
        ..
    } = kind
        && let Some(bind) = typed.defs.get(target)
        && !bind.is_extern
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
                let comparison = match relation {
                    ast::ProofRelation::Equal => crate::typed::MathematicalComparison::Equal,
                    ast::ProofRelation::NotEqual => crate::typed::MathematicalComparison::NotEqual,
                    ast::ProofRelation::Less => crate::typed::MathematicalComparison::Less,
                    ast::ProofRelation::LessOrEqual => {
                        crate::typed::MathematicalComparison::LessOrEqual
                    }
                    ast::ProofRelation::Greater => crate::typed::MathematicalComparison::Greater,
                    ast::ProofRelation::GreaterOrEqual => {
                        crate::typed::MathematicalComparison::GreaterOrEqual
                    }
                };
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
            return Some(crate::typed::ResultEvidence {
                owner: owner.clone(),
                alternatives: evidence,
            });
        }
    }
    if let TypedExprKind::FnCall {
        target,
        args: Some(args),
        ..
    } = kind
        && let Some(bind) = typed.defs.get(target)
        && !bind.is_extern
        && let Some(evidence) = &bind.return_evidence
    {
        let substitute = |operand: ExprId| {
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
        let alternatives = evidence
            .alternatives
            .iter()
            .filter_map(|alternative| {
                let crate::typed::EvidenceProposition::IntegerComparison {
                    lhs,
                    rhs,
                    comparison,
                } = alternative.proposition;
                Some(crate::typed::AlternativeEvidence {
                    label: alternative.label,
                    proposition: crate::typed::EvidenceProposition::IntegerComparison {
                        lhs: substitute(lhs)?,
                        rhs: substitute(rhs)?,
                        comparison,
                    },
                })
            })
            .collect();
        return Some(crate::typed::ResultEvidence {
            owner: evidence.owner.clone(),
            alternatives,
        });
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
    let Ty::ResultFamily { owner, .. } = ty else {
        return None;
    };
    use crate::typed::MathematicalComparison as Math;
    let (taken, rejected) = match predicate {
        crate::intrinsic::ComparisonPredicate::Less => (Math::Less, Math::GreaterOrEqual),
        crate::intrinsic::ComparisonPredicate::LessOrEqual => (Math::LessOrEqual, Math::Greater),
        crate::intrinsic::ComparisonPredicate::Greater => (Math::Greater, Math::LessOrEqual),
        crate::intrinsic::ComparisonPredicate::GreaterOrEqual => (Math::GreaterOrEqual, Math::Less),
        crate::intrinsic::ComparisonPredicate::Equal => (Math::Equal, Math::NotEqual),
    };
    let proposition = |comparison| crate::typed::EvidenceProposition::IntegerComparison {
        lhs: *lhs,
        rhs: *rhs,
        comparison,
    };
    Some(crate::typed::ResultEvidence {
        owner: owner.clone(),
        alternatives: vec![
            crate::typed::AlternativeEvidence {
                label: Intern::from_ref("False"),
                proposition: proposition(rejected),
            },
            crate::typed::AlternativeEvidence {
                label: Intern::from_ref("True"),
                proposition: proposition(taken),
            },
        ],
    })
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
    if registry.nominal_integer_semantics_for_type(ty).is_some() {
        return Some(ast::integer::IntegerKnowledge::Unknown);
    }
    if let Some(validity) = ty.anonymous_integer_validity() {
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
    match expected {
        Ty::UnresolvedLiteral(_) => None,
        Ty::Opaque(name) if name.as_str() == "Int" => None,
        _ => (!matches!(
            registry.resolved_definition_for_type(expected),
            Ty::Opaque(name) if name.as_str().chars().next().is_some_and(char::is_lowercase)
        ))
        .then_some(expected.clone()),
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
    env: &LocalEnv,
    _flaws: &mut Vec<Diagnostic>,
) -> Option<ReferenceTargetSet> {
    let child_group = |id: ExprId| {
        typed
            .exprs
            .target_group
            .get(id.as_usize())
            .cloned()
            .flatten()
    };

    match kind {
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
            let return_group = signature.return_group?;
            typed
                .apply_target_groups(args, signature)
                .targets(return_group)
                .cloned()
        }
        TypedExprKind::Ref(inner)
        | TypedExprKind::TakePtr(inner)
        | TypedExprKind::ConsumeArg(inner)
        | TypedExprKind::Eat(inner) => child_group(*inner),
        TypedExprKind::Reassign { name, .. } => env.target_groups.get(name).cloned(),
        TypedExprKind::Bind { name, .. } => env.target_groups.get(name).cloned(),
        TypedExprKind::Deref(inner) => child_group(*inner).map(|targets| {
            targets.projected(|target| ReferenceTargetGroup::Deref(Box::new(target.clone())))
        }),
        TypedExprKind::TupleGet { base, index } => child_group(*base).map(|targets| {
            targets.projected(|target| ReferenceTargetGroup::Field {
                base: Box::new(target.clone()),
                index: *index,
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

pub(crate) fn target_index_for_expr(typed: &TypedFileAst, expr_id: ExprId) -> TargetIndex {
    let idx = expr_id.as_usize();
    if let Some(value) = typed.exprs.const_value.get(idx).and_then(Clone::clone) {
        return TargetIndex::Symbolic(ast::NormalExpr::Value(value));
    }
    let Some(kind) = typed.exprs.kind.get(idx) else {
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
            return target_index_for_expr(typed, *inner);
        }
        TypedExprKind::Cast { expr, .. } => return target_index_for_expr(typed, *expr),
        TypedExprKind::Rematerialize { source, .. } => {
            return target_index_for_expr(typed, *source);
        }
        TypedExprKind::TargetQuery { kind, operand } => {
            let Some(operand) = target_query_type_reference(operand) else {
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
        TypedExprKind::FnCall {
            target,
            args: Some(args),
            ..
        } if args.len() == 2 => {
            let TargetIndex::Symbolic(left) = target_index_for_expr(typed, args[0]) else {
                return TargetIndex::Unknown;
            };
            let TargetIndex::Symbolic(right) = target_index_for_expr(typed, args[1]) else {
                return TargetIndex::Unknown;
            };
            match target.0.as_str() {
                "add" => ast::NormalExpr::Add(Box::new(left), Box::new(right)),
                "sub" => ast::NormalExpr::Sub(Box::new(left), Box::new(right)),
                "mul" => ast::NormalExpr::Mul(Box::new(left), Box::new(right)),
                _ => return TargetIndex::Unknown,
            }
        }
        _ => return TargetIndex::Unknown,
    };
    TargetIndex::Symbolic(expr)
}

fn target_query_type_reference(ty: &Ty) -> Option<ast::TypeReference> {
    match ty {
        Ty::Unit => Some(ast::TypeReference::Unit),
        Ty::Opaque(name) => Some(ast::TypeReference::Nominal(*name)),
        Ty::Ptr { inner } => Some(ast::TypeReference::Pointer(Box::new(
            target_query_type_reference(inner)?,
        ))),
        Ty::Named { name, instance, .. } => Some(ast::TypeReference::Application {
            name: *name,
            arguments: instance
                .arguments
                .iter()
                .filter_map(|(_, argument)| match argument {
                    ast::TyArg::Type(ty) => target_query_type_reference(ty),
                    ast::TyArg::Const(_) => None,
                })
                .collect(),
        }),
        _ => None,
    }
}

fn when_pattern_key(pattern: &ast::Pattern) -> String {
    pattern_surface(pattern)
}

fn pattern_surface(pattern: &ast::Pattern) -> String {
    match pattern {
        ast::Pattern::Nominal(name, _) => name.as_str().to_string(),
        ast::Pattern::Qualified(path) => {
            let mut out = path.value.root.as_str().to_string();
            for segment in &path.value.segments {
                out.push('.');
                out.push_str(segment.as_str());
            }
            out
        }
        ast::Pattern::Generic { name, params, .. } => {
            let params = params
                .iter()
                .map(|(param_name, kind)| match kind {
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
        ast::Pattern::Tuple(elems) => format!(
            "({})",
            elems
                .iter()
                .map(|elem| pattern_surface(&elem.value))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ast::Pattern::InRange { bounds, .. } => format!("in {bounds}"),
    }
}

fn when_pattern_is_wildcard(pattern: &ast::Pattern) -> bool {
    matches!(pattern, ast::Pattern::Nominal(name, _) if name.as_str() == "_")
}

fn is_lowercase_name(name: &Intern<String>) -> bool {
    name.as_str()
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase())
}

fn const_for_def_id(def_id: DefId, typed: &TypedFileAst) -> Option<ast::ConstValue> {
    let Some(bind) = typed.defs.get(&def_id) else {
        let eval_ast = typed.eval_ast.as_ref();
        let source_bind = eval_ast.defs.get(&def_id.0)?;
        let ast::BindValue::Expr(expr) = &source_bind.value else {
            return None;
        };
        let const_binds = const_env_from_prepared_ast(eval_ast);
        let evaluator = crate::analysis::CompTimeEvaluator::new(&const_binds, eval_ast);
        return expr
            .const_value
            .clone()
            .or_else(|| evaluator.eval(&expr.value));
    };
    let BindBody::Expr(body) = bind.body else {
        return None;
    };
    typed
        .exprs
        .const_value
        .get(body.as_usize())?
        .clone()
        .or_else(|| match typed.exprs.kind.get(body.as_usize())? {
            TypedExprKind::TagCall {
                variant_id, args, ..
            } => Some(ast::ConstValue::Tag {
                name: variant_id.name,
                qual_path: None,
                args: args
                    .as_ref()
                    .map(|args| {
                        args.iter()
                            .filter_map(|arg| {
                                typed
                                    .exprs
                                    .const_value
                                    .get(arg.as_usize())
                                    .cloned()
                                    .flatten()
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            _ => None,
        })
}

/// Try to extract a [`ConstValue`] from a pattern parameter's [`ParameterKind`].
fn param_kind_const(kind: &ParameterKind) -> Option<ast::ConstValue> {
    match kind {
        ParameterKind::Tagged(sp)
        | ParameterKind::ValueParam { ty: sp }
        | ParameterKind::Inferred { ty: sp } => match &sp.value {
            ast::Expr::Lit(lit) => match lit {
                ast::Literal::Int(n) => Some(ast::ConstValue::Int(*n)),
                ast::Literal::Number(n) => Some(ast::ConstValue::Int((*n as u128).into())),
                ast::Literal::String(s) => Some(ast::ConstValue::String(s.clone())),
                ast::Literal::Float(HashFloat(f)) => Some(ast::ConstValue::Float(HashFloat(*f))),
            },
            _ => None,
        },
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
        ast::Pattern::Nominal(name, _) if !is_lowercase_name(name) && name.as_str() != "_" => {
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
        // Generic variant: e.g. `Some(x)` or `Some(5)` — resolve args if all are concrete
        ast::Pattern::Generic { name, params, .. } if typed.variant_map.contains_key(name) => {
            let args: Vec<ast::ConstValue> = params
                .iter()
                .filter_map(|(_, kind)| param_kind_const(kind))
                .collect();
            // Only produce a Tag value if ALL params could be resolved.
            // If any param is a binding (Generic), return None so the
            // reachability check falls through to pattern_matches_public.
            if args.len() == params.len() {
                return Some(ast::ConstValue::Tag {
                    name: *name,
                    qual_path: None,
                    args: args.into(),
                });
            }
        }
        _ => {}
    }
    None
}

fn pattern_matches_variant_with_values(
    pattern: &ast::Pattern,
    variant_name: Intern<String>,
    typed: &TypedFileAst,
) -> bool {
    if pattern.surface_mangle_name() == variant_name.as_str() {
        return true;
    }
    matches!(
        pattern_value_const(pattern, typed),
        Some(ast::ConstValue::Tag { name, .. }) if name == variant_name
    )
}

fn pattern_matches_const_with_values(
    pattern: &ast::Pattern,
    value: &ast::ConstValue,
    typed: &TypedFileAst,
) -> bool {
    crate::analysis::pattern::pattern_matches_public(pattern, value)
        || pattern_value_const(pattern, typed)
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
    let patterns: Vec<_> = arms
        .iter()
        .filter_map(|arm| match arm {
            TypedWhenArm::Is { pattern, .. } => Some(&pattern.value),
            _ => None,
        })
        .collect();
    if patterns.is_empty() {
        return false;
    }
    if patterns
        .iter()
        .any(|pattern| pattern.is_catch_all_pattern())
    {
        return true;
    }
    let subject_definition = typed.type_registry.resolved_definition_for_type(subject_ty);
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
        Ty::Union { variants, .. } => variants.iter().all(|v| {
            patterns
                .iter()
                .any(|pattern| pattern_matches_variant_with_values(pattern, v.name, typed))
        }),
        _ => when_is_exhaustive(Some(&subject_definition), arms),
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
    let mut seen_cond = false;
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
                    flaws.push(Diagnostic::new(
                        "type-unreachable-when-arm",
                        "when arm is unreachable",
                    ));
                }
                seen_cond = true;
            }
            TypedWhenArm::Is { pattern, .. } => {
                if seen_else || seen_catch_all {
                    flaws.push(Diagnostic::new(
                        "type-unreachable-when-arm",
                        "when arm is unreachable",
                    ));
                }
                seen_is = true;
                if when_pattern_is_wildcard(&pattern.value) {
                    flaws.push(Diagnostic::new(
                        "type-wildcard-when-pattern",
                        "wildcard `_` is not allowed in when patterns",
                    ));
                }
                seen_catch_all |= pattern.value.is_catch_all_pattern();
                // Shape-based duplicate detection
                let key = when_pattern_key(&pattern.value);
                if !covered_patterns.insert(key.clone()) {
                    flaws.push(Diagnostic::new(
                        "type-unreachable-when-arm",
                        "when arm is unreachable",
                    ));
                } else if let Some(cv) = pattern_value_const(&pattern.value, typed) {
                    // Semantic duplicate detection: same resolved value
                    if covered_values.iter().any(|v| v == &cv) {
                        flaws.push(Diagnostic::new(
                            "type-unreachable-when-arm",
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
        flaws.push(Diagnostic::new(
            "type-else-not-last",
            "`else` must be the last arm in `when`",
        ));
    }
    if seen_is && (seen_cond || !has_subject) {
        flaws.push(Diagnostic::new(
            "type-mixed-when-forms",
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
                Diagnostic::new(
                    "type-duplicate-field",
                    format!("field `{}` provided twice", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            );
        }
    }

    // 2. Unknown field detection
    for name in field_names.iter() {
        if name.as_str().is_empty() {
            flaws.push(Diagnostic::new(
                "type-expected-field-name",
                "expected field name or `name: expr`".to_string(),
            ));
            continue;
        }
        if !valid_field_set.contains(name) {
            flaws.push(
                Diagnostic::new(
                    "type-unknown-field",
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
                Diagnostic::new(
                    "type-missing-field",
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
                        Diagnostic::new(
                            "type-mismatch",
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
        (Ty::Literal(ConstValue::String(_)), Ty::AnonymousInteger { .. }) => true,
        // Int literal assigned to non-Int field like Bool (union)
        (Ty::Literal(ConstValue::Int(_)), Ty::Union { .. }) => true,
        // String literal assigned to union (Bool is True/False)
        (Ty::Literal(ConstValue::String(_)), Ty::Union { .. }) => true,
        (Ty::Opaque(actual), Ty::Opaque(expected)) => actual != expected,
        _ => false,
    }
}

fn check_type_flaws(
    kind: &TypedExprKind,
    ty: &Ty,
    typed: &TypedFileAst,
    name_span_id: Option<SpanId>,
    locals: &HashSet<Intern<String>>,
    local_var_types: &LocalVarTypes,
    flaws: &mut Vec<Diagnostic>,
) {
    match kind {
        TypedExprKind::TupleAlloc { init, .. } => {
            if let Some((element, _)) =
                crate::representation::Repr::array_parts(ty, Some(&typed.type_registry))
                && let Some(initializer) = typed.exprs.ty.get(init.as_usize())
                && initializer != &element
                && !matches!(initializer, Ty::UnresolvedLiteral(_))
            {
                flaws.push(Diagnostic::new(
                    "type-fixed-array-element-mismatch",
                    "fixed-array initializer does not match the selected element type",
                ));
            }
        }
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            check_fn_call_bounds(
                typed,
                target,
                args.as_deref(),
                substituted_ty.as_ref(),
                flaws,
            );
            let is_known = typed.defs.contains_key(target)
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
                    Diagnostic::new("type-unknown-symbol", format!("unknown symbol `{}`", name))
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
            let is_record = is_record_tag_call(variant_id, typed);

            // Unknown symbol: bare tag that isn't a known record
            let is_result_alternative = matches!(
                ty,
                Ty::ResultFamily { alternatives, .. }
                    if alternatives.iter().any(|alternative| alternative.label == variant_id.name)
            );
            if variant_id.union.0 == variant_id.name && !is_record && !is_result_alternative {
                let mut diag = Diagnostic::new(
                    "type-unknown-symbol",
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
            {
                let tag_id = TagId(variant_id.name);
                if let Some(record_ty) = typed.tag_types.get(&tag_id)
                    && let Ty::Record { fields, .. } =
                        typed.type_registry.resolved_definition_for_type(record_ty)
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
                                "type-equality-mismatched-operands",
                                format!(
                                    "equality requires operands of the same type, found `{}` and `{}`",
                                    lhs_ty.format_for_hover(),
                                    rhs_ty.format_for_hover()
                                ),
                            ),
                            crate::equality::EqualityError::UnsupportedType => (
                                "type-equality-unsupported",
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
                    flaws.push(Diagnostic::new("type-mismatch", "type mismatch"));
                }
            }
        }
        TypedExprKind::When(when_expr) => {
            validate_when_arm_shape(&when_expr.arms, when_expr.subject.is_some(), typed, flaws);
            let has_else = when_expr
                .arms
                .iter()
                .any(|arm| matches!(arm, TypedWhenArm::Else(..)));
            let subject_ty = when_expr
                .subject
                .and_then(|id| typed.exprs.ty.get(id.as_usize()));
            let exhaustive =
                when_is_exhaustive_with_value_patterns(subject_ty, &when_expr.arms, typed);
            if !has_else && !exhaustive {
                flaws.push(Diagnostic::new(
                    "type-missing-else-arm",
                    "`when` should have an `else` arm",
                ));
            } else if has_else && exhaustive {
                flaws.push(Diagnostic::new(
                    "type-unreachable-else-arm",
                    "`else` arm is unreachable",
                ));
            }
            // Subject-value reachability: if the subject has a known compile-time value,
            // check which arms match and flag unmatched/after-match arms as unreachable.
            if let Some(subject_id) = when_expr.subject
                && let Some(Some(subject_cv)) = typed.exprs.const_value.get(subject_id.as_usize())
            {
                let mut matching_arm_found = false;
                for arm in &when_expr.arms {
                    match arm {
                        TypedWhenArm::Is { pattern, .. } => {
                            if matching_arm_found {
                                flaws.push(Diagnostic::new(
                                    "type-unreachable-when-arm",
                                    "when arm is unreachable",
                                ));
                            } else if let Some(pattern_cv) =
                                pattern_value_const(&pattern.value, typed)
                            {
                                // Pattern resolves to a specific const value
                                if &pattern_cv != subject_cv {
                                    flaws.push(Diagnostic::new(
                                        "type-unreachable-when-arm",
                                        "when arm is unreachable",
                                    ));
                                } else {
                                    matching_arm_found = true;
                                }
                            } else if !crate::analysis::pattern::pattern_matches_with_tag_types(
                                &pattern.value,
                                subject_cv,
                                &typed.tag_types,
                            ) {
                                // Pattern can be statically compared against subject value
                                // (handles literal, InRange, uppercase variant patterns)
                                flaws.push(Diagnostic::new(
                                    "type-unreachable-when-arm",
                                    "when arm is unreachable",
                                ));
                            } else {
                                matching_arm_found = true;
                            }
                        }
                        TypedWhenArm::Else(..) => {
                            if matching_arm_found {
                                flaws.push(Diagnostic::new(
                                    "type-unreachable-else-arm",
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
                let actual = match typed.exprs.integer_knowledge[expr.as_usize()].as_ref() {
                    Some(ast::integer::IntegerKnowledge::Exact(
                        ast::integer::CanonicalIntegerExpr::Value(value),
                    )) => ast::integer::IntegerDomain::bounded(*value, *value),
                    Some(ast::integer::IntegerKnowledge::Domain(domain)) => Some(domain.clone()),
                    Some(ast::integer::IntegerKnowledge::Exact(
                        ast::integer::CanonicalIntegerExpr::Symbolic(_),
                    ))
                    | Some(ast::integer::IntegerKnowledge::Unknown)
                    | None => typed
                        .type_registry
                        .integer_validity_for_type(source)
                        .map(|validity| validity.domain().clone()),
                    Some(ast::integer::IntegerKnowledge::Poison) => None,
                };
                let proof = actual
                    .as_ref()
                    .map_or(ast::integer::ContainmentProof::Unknown, |actual| {
                        ast::integer::prove_containment(actual, expected.domain())
                    });
                if let Some(code) = proof.diagnostic_code() {
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
                    Diagnostic::new(
                        "type-unsupported-address-space",
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
                        Diagnostic::new(
                            "type-invalid-address-conversion-width",
                            "raw address conversion requires a 32-bit or 64-bit integer",
                        )
                        .with_arg("width", width.to_string()),
                    );
                }
            }
        }
        TypedExprKind::TakePtr(inner) => {
            let pointee = typed.exprs.ty.get(inner.as_usize());
            if let Some(address_space) = ty.address_space() {
                if let Some(expected_pointee) = ty.pointee_ty()
                    && pointee.is_some_and(|actual| actual != expected_pointee)
                {
                    flaws.push(
                        Diagnostic::new(
                            "type-raw-pointer-pointee-mismatch",
                            "address-taking pointee does not match the expected raw-pointer type",
                        )
                        .with_arg("address_space", address_space.to_string()),
                    );
                }
            } else if !ty.is_unresolved_address() {
                flaws.push(Diagnostic::new(
                    "type-address-taking-non-pointer",
                    "address-taking requires a raw-pointer representation",
                ));
            }
        }
        TypedExprKind::Deref(inner) => {
            if typed
                .exprs
                .ty
                .get(inner.as_usize())
                .is_none_or(|inner_ty| inner_ty.pointee_ty().is_none())
            {
                flaws.push(Diagnostic::new(
                    "type-deref-non-address",
                    "cannot dereference a value without raw address representation",
                ));
            }
        }
        TypedExprKind::Destructure {
            tag_name,
            value,
            field_bindings,
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
                                Diagnostic::new(
                                    "type-unknown-variant",
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
                            Diagnostic::new(
                                "type-unknown-field",
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
                    BindBody::Extern => None,
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
                    Diagnostic::new(
                        "type-unknown-field",
                        format!("record has no field `{}`", field.as_str()),
                    )
                    .with_arg("name", field.as_str().to_string()),
                );
            }
        }
        _ => {}
    }
}
