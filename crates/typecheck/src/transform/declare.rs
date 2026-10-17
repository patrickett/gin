//! Stage 1: Declare — Resolve tag/bind declarations, build file-level index.
//!
//! This stage walks the `FileAst` tags and defs, translates them into
//! [`TypedTag`] and [`TypedBind`], populates `tag_types`, `fn_return_types`,
//! and `variant_map`, and detects declaration-level flaws.

use crate::tag_declarations::TagDeclarations;
use diagnostic::code::{
    ArrayCode, CallCode, GenericCode, IntegerCode, MemberCode, OperatorCode, OwnershipCode,
    ProofCode, ReferenceCode, ResultCode, SymbolCode, TargetGroupCode, TraitCode, TypeCode,
};
use internment::Intern;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::Arc;

/// Enough for mutual refs (e.g. `Type` ↔ `NamedTy`); keep low — each pass re-resolves every tag.
const TAG_RESOLVE_MAX_PASSES: usize = 8;

use diagnostic::{Diagnostic, DiagnosticCode};

use super::collect_result_exit_alternatives;
use super::lower_ty::nominalize_explicit_ty_surface;
use super::proof_walk::read::{ProofVisitor, walk_normal, walk_reference, walk_term};
use super::{TransformCtx, flaw};
use crate::analysis::expr_is_type_surface;
use crate::analysis::{TyInfer, TyInferEnv, TypeEnv};
use crate::representation::RepresentationError;
use crate::ty::{Ty, TyChild, UnionVariant, reference_semantics_for_type};
use crate::type_registry::ReferenceSemantics;
use crate::typed::{
    AvailabilityRequirement, CallCapability, FileId, GroupId, GroupProjection, TagId, TypedBind,
    TypedFileAst, TypedGroup, TypedTag, VariantMap,
};
use ast::integer::{IntegerDomain, IntegerValidity};
use ast::parameter::Parameters;
use ast::prelude::*;
use ast::type_decl::TypeNameExt;
use ast::{
    BindValue, BinderId, BinderOwner, ConstValue, DeclareValue, HashFloat, ImportSource, Literal,
    ParamKind, Pattern,
};

use ast_format::declare::{BindFormatExt, DeclareFormatExt};
use ast_format::type_expr::{ExprFormatExt, PatternFormatExt};

fn nominal_integer_semantics(
    ty: &Ty,
    attributes: &ast::DeclareAttributes,
) -> Option<ast::integer::NominalIntegerSemantics> {
    integer_semantics(ty.anonymous_integer_validity()?, attributes).into()
}

fn checked_integer_operator_result(
    registry: &crate::TypeRegistry,
    operand: &Ty,
    result: &Ty,
) -> bool {
    let Ty::Union { variants, .. } = registry.resolved_definition_for_type(result) else {
        return false;
    };
    variants.len() == 2 && variants.iter().any(|variant| {
        variant.kind == ast::ty::UnionVariantKind::Type
            && matches!(variant.fields.as_slice(), [(_, payload)] if payload.as_ref() == operand)
    }) && variants.iter().any(|variant| {
        variant.kind == ast::ty::UnionVariantKind::Constructor
            && variant.name.as_str().rsplit('.').next() == Some("Flaw")
    })
}

fn integer_semantics(
    validity: &IntegerValidity,
    attributes: &ast::DeclareAttributes,
) -> ast::integer::NominalIntegerSemantics {
    let interpretation = if validity
        .domain()
        .storage_hull()
        .is_some_and(|hull| hull.min().is_negative())
    {
        ast::integer::IntegerInterpretation::Signed
    } else {
        ast::integer::IntegerInterpretation::Unsigned
    };
    let representation_rule = attributes.bits.as_ref().map_or(
        ast::integer::IntegerRepresentationRule::InferFromValidity,
        |bits| {
            let natural = if let Some(ast::ConstValue::Int(value)) = bits.const_value.as_deref() {
                ast::integer::to_u64(*value)
                    .and_then(|value| u32::try_from(value).ok())
                    .and_then(std::num::NonZeroU32::new)
                    .map(ast::integer::CanonicalNaturalExpr::Value)
                    .unwrap_or_else(|| {
                        ast::integer::CanonicalNaturalExpr::Symbolic(ast::NormalExpr::from(*value))
                    })
            } else {
                match &bits.value {
                    ast::Expr::Lit(ast::Literal::Int(value)) => ast::integer::to_u64(*value)
                        .and_then(|value| u32::try_from(value).ok())
                        .and_then(std::num::NonZeroU32::new)
                        .map(ast::integer::CanonicalNaturalExpr::Value)
                        .unwrap_or_else(|| {
                            ast::integer::CanonicalNaturalExpr::Symbolic(ast::NormalExpr::from(
                                *value,
                            ))
                        }),
                    ast::Expr::FnCall(call) if call.args.is_none() => {
                        ast::integer::CanonicalNaturalExpr::Symbolic(ast::NormalExpr::Var(
                            Intern::new(call.path.value.to_qualified_string()),
                        ))
                    }
                    _ => ast::integer::CanonicalNaturalExpr::Symbolic(ast::NormalExpr::Var(
                        Intern::from_ref("<invalid bits>"),
                    )),
                }
            };
            ast::integer::IntegerRepresentationRule::ExactBits(natural)
        },
    );
    ast::integer::NominalIntegerSemantics {
        validity: validity.clone(),
        representation_rule,
        interpretation,
    }
}

/// Takes `&FileAst` and returns a `TypedFileAst` with declarations
/// resolved and the file-level index populated. The expression arena is left
/// empty — it will be filled in Stage 2.
pub fn stage_declare(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    declare_signatures(declare_types(file_ast, file_id, ctx), ctx)
}

#[derive(Clone)]
pub(super) struct DeclaredTypes<'a> {
    pub typed: TypedFileAst,
    source: DeclarationSource<'a>,
    tag_params: HashMap<Intern<String>, Parameters>,
    anonymous_result_labels: HashSet<Intern<String>>,
    target_dependent_names: Option<HashSet<Intern<String>>>,
}

#[derive(Clone)]
enum DeclarationSource<'a> {
    Borrowed(&'a FileAst),
    Shared(Arc<FileAst>),
}

impl AsRef<FileAst> for DeclarationSource<'_> {
    fn as_ref(&self) -> &FileAst {
        match self {
            Self::Borrowed(source) => source,
            Self::Shared(source) => source,
        }
    }
}

impl DeclaredTypes<'_> {
    pub(super) fn same(&self, other: &Self) -> bool {
        let Self {
            typed,
            source,
            tag_params,
            anonymous_result_labels,
            target_dependent_names,
        } = self;
        super::check_session::file_eq(typed, &other.typed, false)
            && source.as_ref() == other.source.as_ref()
            && super::check_session::same_ast_order(source.as_ref(), other.source.as_ref())
            && tag_params == &other.tag_params
            && anonymous_result_labels == &other.anonymous_result_labels
            && target_dependent_names == &other.target_dependent_names
    }

    pub(super) fn snapshot(&self) -> DeclaredTypes<'static> {
        self.snapshot_with_source(match &self.source {
            DeclarationSource::Borrowed(source) => Arc::new((*source).clone()),
            DeclarationSource::Shared(source) => Arc::clone(source),
        })
    }

    pub(super) fn snapshot_with_source(&self, source: Arc<FileAst>) -> DeclaredTypes<'static> {
        DeclaredTypes {
            typed: self.typed.clone(),
            source: DeclarationSource::Shared(match &self.source {
                DeclarationSource::Borrowed(_) => source,
                DeclarationSource::Shared(source) => Arc::clone(source),
            }),
            tag_params: self.tag_params.clone(),
            anonymous_result_labels: self.anonymous_result_labels.clone(),
            target_dependent_names: self.target_dependent_names.clone(),
        }
    }
}

pub(super) fn declare_types<'a>(
    file_ast: &'a FileAst,
    file_id: FileId,
    ctx: &TransformCtx,
) -> DeclaredTypes<'a> {
    declare_types_with_aliases(file_ast, file_id, ctx, &HashMap::new())
}

pub(super) fn declare_types_with_aliases<'a>(
    file_ast: &'a FileAst,
    file_id: FileId,
    ctx: &TransformCtx,
    completed_aliases: &HashMap<TagId, Ty>,
) -> DeclaredTypes<'a> {
    let _profile = compiler_profile::span(compiler_profile::Phase::Declarations);
    let defaults =
        match super::trait_methods::TraitMethods::new(file_ast, &ctx.compile_time_eval_ast)
            .expand_defaults()
        {
            std::borrow::Cow::Borrowed(_) => DeclarationSource::Borrowed(file_ast),
            std::borrow::Cow::Owned(expanded) => DeclarationSource::Shared(Arc::new(expanded)),
        };
    let file_ast = defaults.as_ref();
    let mut typed = TypedFileAst::new(file_id, file_ast.span_table.clone());
    super::bindings::validate(file_ast, &mut typed);
    typed.target_layout =
        crate::layout::TargetLayout::from_compile_target(&ctx.compile_target).ok();
    let anonymous_result_labels = anonymous_result_labels(file_ast);
    typed.semantic_origin = Some(crate::typed::Shared::new(
        file_ast
            .semantic_origin
            .clone()
            .unwrap_or_else(|| ast::prelude::FileSemanticOrigin {
                package: ctx.package_instance.clone(),
                module: Vec::new(),
            }),
    ));
    typed.overload_groups = crate::typed::Shared::new(file_ast.defs.groups().clone());
    typed.module_doc = file_ast.module_doc.clone().map(crate::typed::Shared::new);
    typed.type_registry.merge_from(&ctx.type_registry);
    for declare in file_ast.tags.values() {
        let ast::DeclareValue::Has(members) = &declare.value else {
            continue;
        };
        for member in members {
            let ast::HasMember::Function(function) = member else {
                continue;
            };
            validate_payload_names(function, &mut typed.declaration_flaws, &typed.span_table);
            let self_name = Intern::<String>::from_ref("self");
            if !function.params.contains_key(&self_name) {
                continue;
            }
            let Some(body) = function.body.as_ref() else {
                continue;
            };
            let value = match body {
                ast::HasMemberBody::Overrideable(value) | ast::HasMemberBody::Final(value) => value,
            };
            let modifier = match function.conventions.get(&self_name) {
                Some(ast::ParamConvention::Observe) => "self ref",
                Some(ast::ParamConvention::Mutate) => "self mut",
                Some(ast::ParamConvention::Own) => "self own",
                Some(ast::ParamConvention::Terminal) | None => "self",
            };
            let mut spans = Vec::new();
            collect_bind_value_self_ref_spans(value, &mut spans);
            typed.self_contexts.extend(
                spans
                    .into_iter()
                    .map(|span| (span, declare.name, Intern::<String>::from_ref(modifier))),
            );
        }
    }

    // Collect raw ModPaths from imports for module path hover detection.
    for import in &file_ast.uses {
        for mi in &import.0 {
            if let ast::ImportSource::Package(mp) = &mi.source {
                typed.import_mod_paths.push(mp.clone());
            } else if let ast::ImportSource::LocalBundle(bundle) = &mi.source
                && let Some(path) = &bundle.module_path
            {
                typed.import_mod_paths.push(path.as_ref().clone());
            }
        }
    }
    typed.imported_trait_names = crate::typed::Shared::new(file_ast.imported_trait_names());
    typed.eval_ast = Arc::clone(&ctx.compile_time_eval_ast);
    let target_dependent_names = matches!(ctx.compile_target, flask::CompileTarget::Library)
        .then(|| crate::prepare_target::target_dependent_bind_names(&ctx.compile_time_eval_ast));

    // 1. Walk tag declarations and resolve them (fixpoint for forward refs in the same file).
    let mut tag_params = tag_params_map(&file_ast.tags);
    // Merge cross-file tag params (e.g. List(x) from marker package) so that
    // generic parameter substitution works for types like List(NamedTy).
    for (name, decl) in &ctx.compile_time_eval_ast.tags {
        if let Some(p) = &decl.params {
            tag_params.entry(*name).or_insert_with(|| p.clone());
        }
    }
    let domain_declarations =
        TagDeclarations::overlay(&file_ast.tags, &ctx.compile_time_eval_ast.tags);
    let resolved_tags = resolve_tags_fixpoint(
        &file_ast.tags,
        domain_declarations,
        &tag_params,
        &ctx.cross_file_tag_types,
        file_id,
        TAG_RESOLVE_MAX_PASSES,
        completed_aliases,
    );
    let available_tags = nominal_types_for_definitions(&resolved_tags, &file_ast.tags, file_id);
    for (name, declare) in &file_ast.tags {
        let is_transparent_alias = matches!(&declare.value, ast::DeclareValue::Alias(alias)
            if !matches!(&alias.value, ast::Expr::TakePtr(_) | ast::Expr::IntegerDomain(_))
        );
        if is_transparent_alias {
            continue;
        }
        let Some(definition) = resolved_tags.get(name) else {
            continue;
        };
        let handle = ast::ty::TypeId {
            file: file_id.0,
            name: *name,
        };
        let key = file_ast.semantic_origin.as_ref().map_or_else(
            || ast::ty::DeclarationKey::from_qualified_name(ctx.package_instance.clone(), *name),
            |origin| {
                ast::ty::DeclarationKey::new(
                    origin.package.clone(),
                    origin.module.clone(),
                    declare.name,
                )
            },
        );
        typed.type_registry.insert(
            handle,
            crate::TypeDeclarationSemantics {
                key,
                display_name: *name,
                parameters: declare
                    .params
                    .as_ref()
                    .map(|parameters| parameters.keys().copied().collect())
                    .unwrap_or_default(),
                definition: definition.clone(),
                sequence_literal: validated_sequence_literal_acceptance(declare),
                private_members: member_visibility(declare).0,
                private_fields: member_visibility(declare).1,
                field_refinements: field_refinements(declare),
                reference: None,
                integer: nominal_integer_semantics(definition, &declare.attributes),
                discriminant: definition.union_discriminant_domain().map(|domain| {
                    integer_semantics(&IntegerValidity::new(domain), &declare.attributes)
                }),
            },
        );
        if target_dependent_names
            .as_ref()
            .is_some_and(|names| declaration_depends_on_target(declare, names))
        {
            typed.type_registry.defer_integer_layout(handle);
        }
    }
    for (name, declare) in &file_ast.tags {
        let tag_id = TagId(*name);
        if file_ast.private_tags.contains(name) {
            typed.private_tags.insert(tag_id);
        }
        let definition = resolved_tags.get(name).cloned().unwrap_or_else(|| {
            resolve_tag_pass(
                name,
                declare,
                (&file_ast.tags).into(),
                &tag_params,
                &ctx.cross_file_tag_types,
                &available_tags,
                file_id,
            )
        });
        let is_transparent_alias = matches!(&declare.value, ast::DeclareValue::Alias(alias)
            if !matches!(&alias.value, ast::Expr::TakePtr(_) | ast::Expr::IntegerDomain(_))
        );
        let instance = ast::ty::NamedTypeInstance::new(ast::ty::TypeId {
            file: file_id.0,
            name: *name,
        })
        .with_arguments(declaration_instance_arguments(declare));
        let resolved_ty = if is_transparent_alias {
            definition.clone()
        } else {
            Ty::Named {
                instance,
                name: *name,
            }
        };
        if !file_ast.private_tags.contains(name)
            && contains_unresolved_result_label(&definition, &anonymous_result_labels)
        {
            typed.declaration_flaws.push((
                declare.name_span,
                anonymous_result_annotation_diagnostic(declare.name_span, &typed.span_table),
            ));
        }
        if let ast::DeclareValue::InRange(start, end) = &declare.value
            && start > end
        {
            typed.declaration_flaws.push((
                declare.name_span,
                // type-integer-domain-empty
                Diagnostic::new(
                    IntegerCode::DomainEmpty,
                    format!(
                        "integer domain `{start} to {end}` is empty",
                        start = start,
                        end = end
                    ),
                )
                .with_arg("min", start.to_string())
                .with_arg("max", end.to_string())
                .at_span_id(declare.name_span, &typed.span_table),
            ));
        }

        typed
            .declaration_flaws
            .extend(crate::discriminant::validate_declaration(
                declare,
                &definition,
                &typed.span_table,
            ));
        let discriminant = definition
            .union_discriminant_domain()
            .map(|domain| integer_semantics(&IntegerValidity::new(domain), &declare.attributes));
        let sequence_literal = validated_sequence_literal_acceptance(declare);
        let integer = nominal_integer_semantics(&definition, &declare.attributes);
        if declare.attributes.bits_span.is_some()
            && !target_dependent_names
                .as_ref()
                .is_some_and(|names| declaration_depends_on_target(declare, names))
            && let Some(integer) = integer.as_ref().or(discriminant.as_ref())
            && let Err(error) = integer
                .validity
                .resolve_runtime_representation(&integer.representation_rule)
        {
            typed.declaration_flaws.push((
                declare.attributes.bits_span.unwrap_or(declare.name_span),
                Diagnostic::new(error.diagnostic_code(), format!("{error:?}")).at_span_id(
                    declare.attributes.bits_span.unwrap_or(declare.name_span),
                    &typed.span_table,
                ),
            ));
        }
        let reference = match declaration_reference_semantics(&definition, &typed.type_registry) {
            Ok(reference) => reference,
            Err(diagnostic) => {
                typed.declaration_flaws.push((
                    declare.name_span,
                    diagnostic.at_span_id(declare.name_span, &typed.span_table),
                ));
                None
            }
        };
        if is_transparent_alias {
            let mut recursion_stack = HashSet::new();
            if declaration_recursively_refers_to_name(
                &declare.name,
                &file_ast.tags,
                &resolved_tags,
                &mut recursion_stack,
            ) {
                let type_id = resolved_ty.type_id().unwrap_or(ast::ty::TypeId {
                    file: file_id.0,
                    name: declare.name,
                });
                typed.declaration_flaws.push((
                    declare.name_span,
                    RepresentationError::RecursiveRepresentation { type_id }
                        .diagnostic()
                        .with_arg("declaration", declare.name.as_str().to_string())
                        .with_arg("type_name", declare.name.as_str().to_string())
                        .at_span_id(declare.name_span, &typed.span_table),
                ));
            }
        }
        if let Some(instance) = resolved_ty.type_id() {
            let key = file_ast.semantic_origin.as_ref().map_or_else(
                || {
                    ast::ty::DeclarationKey::from_qualified_name(
                        ctx.package_instance.clone(),
                        *name,
                    )
                },
                |origin| {
                    ast::ty::DeclarationKey::new(
                        origin.package.clone(),
                        origin.module.clone(),
                        declare.name,
                    )
                },
            );
            if !is_transparent_alias {
                typed.type_registry.insert(
                    instance,
                    crate::TypeDeclarationSemantics {
                        key,
                        display_name: *name,
                        parameters: declare
                            .params
                            .as_ref()
                            .map(|parameters| parameters.keys().copied().collect())
                            .unwrap_or_default(),
                        definition: definition.clone(),
                        sequence_literal,
                        private_members: member_visibility(declare).0,
                        private_fields: member_visibility(declare).1,
                        field_refinements: field_refinements(declare),
                        reference,
                        integer,
                        discriminant,
                    },
                );
            }
            if declare.attributes.invalid_fixed_array.is_some()
                || (declare.attributes.fixed_array.is_some()
                    && typed
                        .type_registry
                        .declaration(instance)
                        .is_some_and(|declaration| declaration.sequence_literal.is_none()))
            {
                typed.declaration_flaws.push((
                    declare.name_span,
                    // type-invalid-fixed-array-former
                    Diagnostic::new(
                    ArrayCode::FixedInvalidFormer,
                    "fixed-array former must name one element and one length parameter belonging to the declaration",
                    )
                    .at_span_id(declare.name_span, &typed.span_table),
                ));
            }
        }
        if resolved_ty.named_instance().is_none()
            && let ast::DeclareValue::Alias(alias) = &declare.value
            && !matches!(
                alias.value,
                ast::Expr::TakePtr(_) | ast::Expr::IntegerDomain(_)
            )
            && let Err(diagnostic) =
                declaration_reference_semantics(&resolved_ty, &typed.type_registry)
        {
            typed.declaration_flaws.push((
                declare.name_span,
                diagnostic.at_span_id(declare.name_span, &typed.span_table),
            ));
        }

        check_qualified_has_properties(declare, &mut |span, diagnostic| {
            typed
                .declaration_flaws
                .push((span, diagnostic.at_span_id(span, &typed.span_table)));
        });

        let provided_traits = declare.provided_traits.clone();

        // Register provision expressions so references inside qualified trait members
        // remain available to hover and go-to-definition.
        let record_fields = if let Ty::Record { fields, .. } = &definition {
            Some(fields)
        } else {
            None
        };
        for pt in &provided_traits {
            for (_, expr) in &pt.fields {
                if expr.span_id.is_valid() {
                    // Try to resolve the expression type for better hover.
                    // If the expression is `self.field`, look up the field type.
                    let expr_ty: Ty = match &expr.value {
                        ast::Expr::RecordGet { field, .. } => {
                            if let Some(fields) = record_fields {
                                fields
                                    .iter()
                                    .find(|(n, _)| n.as_str() == field.as_str())
                                    .map(|(_, ty)| (**ty).clone())
                                    .unwrap_or(resolved_ty.clone())
                            } else {
                                resolved_ty.clone()
                            }
                        }
                        _ => resolved_ty.clone(),
                    };
                    typed.exprs.push_placeholder(
                        crate::typed::TypedExprKind::Lit(ast::Literal::Int(0.into())),
                        expr_ty,
                        expr.span_id,
                    );
                }
            }
        }
        // Extract field/method type annotations from `has` bodies.
        let (record_field_types, record_field_docs, record_field_refinements) = match &declare.value
        {
            DeclareValue::Has(members) => {
                let mut map = HashMap::new();
                let mut doc_map = HashMap::new();
                let mut refinement_map = HashMap::new();
                for m in members {
                    use std::fmt::Write;
                    let mut ty_surface = String::new();
                    match m {
                        ast::HasMember::Property(p) => {
                            if p.qualifier
                                .as_ref()
                                .is_some_and(|qualifier| qualifier.name != declare.name)
                            {
                                continue;
                            }
                            if p.owned_storage {
                                ty_surface.push_str("own ");
                            }
                            if let Some(ty) = &p.ty {
                                let _ = write!(&mut ty_surface, "{}", ty.value.format_surface());
                            }
                            map.insert(p.name, ty_surface);
                            if let Some(doc) = &p.doc_comment
                                && !doc.value.is_empty()
                            {
                                doc_map.insert(p.name, doc.value.clone());
                            }
                            if let Some(refinement) = &p.refinement {
                                refinement_map.insert(p.name, refinement.clone());
                            }
                        }
                        ast::HasMember::Function(f) => {
                            // Function: `name(params...) RetType`
                            if !f.params.is_empty() {
                                ty_surface.push('(');
                                let mut first = true;
                                for (param_name, kind) in f.params.iter() {
                                    if !first {
                                        ty_surface.push_str(", ");
                                    }
                                    first = false;
                                    let _ = write!(&mut ty_surface, "{}", param_name.as_str());
                                    match &kind.kind {
                                        ast::ParameterKind::Tagged(sp) => {
                                            ty_surface.push(' ');
                                            ty_surface.push_str(&sp.value.format_surface());
                                        }
                                        ast::ParameterKind::ValueParam { ty } => {
                                            ty_surface.push(' ');
                                            ty_surface.push_str(&ty.value.format_surface());
                                        }
                                        ast::ParameterKind::Inferred { ty } => {
                                            ty_surface.push(' ');
                                            ty_surface.push_str(&ty.value.format_surface());
                                            ty_surface.push_str(": ?");
                                        }
                                        ast::ParameterKind::Default(expr) => {
                                            let _ = write!(&mut ty_surface, ": {:?}", expr);
                                        }
                                        ast::ParameterKind::Generic
                                        | ast::ParameterKind::Proposition(_) => {}
                                    }
                                    if let Some(conv) = f.conventions.get(param_name) {
                                        match conv {
                                            ast::ParamConvention::Observe => {
                                                ty_surface.push_str(" ref")
                                            }
                                            ast::ParamConvention::Mutate => {
                                                ty_surface.push_str(" mut")
                                            }
                                            ast::ParamConvention::Own => {
                                                ty_surface.push_str(" own")
                                            }
                                            ast::ParamConvention::Terminal => {}
                                        }
                                    }
                                }
                                ty_surface.push(')');
                            }
                            if let Some(rt) = &f.return_ty {
                                if !ty_surface.is_empty() {
                                    ty_surface.push(' ');
                                }
                                ty_surface.push_str(&rt.value.format_surface());
                            }
                            if ty_surface.is_empty() {
                                ty_surface.push_str("()");
                            }
                            map.insert(f.name, ty_surface);
                            if let Some(doc) = &f.doc_comment
                                && !doc.value.is_empty()
                            {
                                doc_map.insert(f.name, doc.value.clone());
                            }
                            if let Some(refinement) = &f.refinement {
                                refinement_map.insert(f.name, refinement.clone());
                            }
                        }
                    }
                }
                (map, doc_map, refinement_map)
            }
            _ => (HashMap::new(), HashMap::new(), HashMap::new()),
        };

        let typed_tag = TypedTag {
            name_span: declare.name_span,
            span: declare.span,
            resolved_ty: resolved_ty.clone(),
            attributes: declare.attributes.clone(),
            doc_comment: declare.doc_comment.clone(),
            record_field_docs,
            record_field_refinements,
            params: declare.params.clone(),
            record_field_types,
            declaration_text: declare.surface_text(),
            provided_traits,
        };
        typed.tags.insert(tag_id, typed_tag);
        typed.tag_types.insert(tag_id, resolved_ty.clone());
    }

    // 2. Build own-file variant map from union tags.
    let own_unions: HashSet<Intern<String>> = file_ast
        .tags
        .iter()
        .filter(|(_, d)| matches!(d.value, DeclareValue::Union { .. }))
        .map(|(n, _)| *n)
        .collect();
    rebuild_own_variant_map(
        &mut typed.variant_map,
        &typed.tag_types,
        &own_unions,
        &typed.type_registry,
    );

    // 2a. Populate variant annotations (shape text, doc comments) from AST variants.
    for (tag_name, declare) in &file_ast.tags {
        if let DeclareValue::Union { variants } = &declare.value {
            for variant in variants {
                let vname = match &variant.shape().value {
                    Pattern::Nominal(name, _) => name.as_str(),
                    Pattern::Generic { name, .. } => name.as_str(),
                    Pattern::Qualified(path) => path
                        .segments
                        .last()
                        .map(|s| s.as_str())
                        .unwrap_or_else(|| path.root.as_str()),
                    _ => continue,
                };
                let key = format!("{}.{}", tag_name.as_str(), vname);
                let doc = variant.doc_comment().map(|d| d.value.clone());
                let shape = variant.shape().value.format_variant_shape();
                typed.variant_annotations.entry(key).or_insert((shape, doc));
            }
        }
    }

    DeclaredTypes {
        typed,
        source: defaults,
        tag_params,
        anonymous_result_labels,
        target_dependent_names,
    }
}

pub(super) fn complete_signatures(
    mut declarations: DeclaredTypes<'_>,
    ctx: &TransformCtx,
) -> TypedFileAst {
    let _profile = compiler_profile::span(compiler_profile::Phase::Signatures);
    declarations.typed.type_registry = ctx.type_registry.clone();
    declarations.typed.variant_map.clear();
    declare_signatures(declarations, ctx)
}

fn declare_signatures(declarations: DeclaredTypes<'_>, ctx: &TransformCtx) -> TypedFileAst {
    let DeclaredTypes {
        mut typed,
        source,
        tag_params,
        anonymous_result_labels,
        target_dependent_names,
    } = declarations;
    let file_ast = source.as_ref();
    let argument_declarations =
        TagDeclarations::overlay(&file_ast.tags, &ctx.compile_time_eval_ast.tags);
    let argument_context =
        super::type_arguments::TypeArgumentContext::new(&tag_params, argument_declarations);

    for bind in file_ast.defs.values() {
        check_params_for_in_range_on_bounded_tag(
            bind.params.as_ref(),
            &typed.tag_types,
            &mut typed.declaration_flaws,
        );
    }

    // 3. Merge cross-file variant map entries (not spliced into own to avoid O(n²)).
    for (variant_name, entries) in &ctx.cross_file_variant_map {
        typed
            .variant_map
            .entry(*variant_name)
            .or_default()
            .extend(entries.iter().cloned());
    }

    let trait_methods =
        super::trait_methods::TraitMethods::new(file_ast, &ctx.compile_time_eval_ast);
    trait_methods.check_implementations(|span, diagnostic| {
        typed
            .declaration_flaws
            .push((span, diagnostic.at_span_id(span, &typed.span_table)));
    });

    let shared_tag_types = std::cell::OnceCell::new();
    // 4. Walk binds and build bind index.
    for (name, declare) in &file_ast.defs {
        let effective = trait_methods.public_signature(declare);
        let declare = effective.as_ref();
        let def_id = *name;
        if file_ast.private_defs.contains(&name.0) {
            typed.private_defs.insert(def_id);
        }
        let dependent_binder = BinderId::new(typed.file_id.0, BinderOwner::Definition(def_id));
        let shared = shared_tag_types.get_or_init(|| {
            let types = signature_tag_types(&typed.tag_types, &ctx.cross_file_tag_types);
            (!types.values().any(|ty| matches!(ty, Ty::Union { .. }))).then_some(types)
        });
        let tag_types_by_name = match shared {
            Some(types) => std::borrow::Cow::Borrowed(types),
            None => std::borrow::Cow::Owned(signature_tag_types(
                &typed.tag_types,
                &ctx.cross_file_tag_types,
            )),
        };
        let receiver_type = declare.receiver_type_surface().map(|receiver| {
            TypeEnv::new(&tag_types_by_name)
                .with_tag_params(&tag_params)
                .with_tag_decls(&file_ast.tags)
                .with_dependent_binder(dependent_binder)
                .resolve_expr(&receiver.value)
        });
        let method_typevar_subst = declare
            .receiver_type_surface()
            .and_then(|receiver| {
                let ast::Expr::TagCall(call) = &receiver.value else {
                    return None;
                };
                Some(
                    call.args
                        .iter()
                        .filter_map(|arg| match &arg.value {
                            ast::Expr::AnonymousTag(name)
                                if name.as_str().chars().next().is_some_and(char::is_lowercase) =>
                            {
                                Some((*name, Ty::Opaque(*name)))
                            }
                            _ => None,
                        })
                        .collect::<HashMap<_, _>>(),
                )
            })
            .unwrap_or_default();
        let result_family_owner = file_ast.semantic_origin.as_ref().map_or_else(
            || {
                ast::ResultFamilyOwner::Callable(Box::new(
                    ast::ty::DeclarationKey::from_qualified_name(
                        ctx.package_instance.clone(),
                        name.0,
                    )
                    .with_member(name.1),
                ))
            },
            |origin| {
                ast::ResultFamilyOwner::Callable(Box::new(
                    ast::ty::DeclarationKey::new(
                        origin.package.clone(),
                        origin.module.clone(),
                        declare.name,
                    )
                    .with_member(name.1),
                ))
            },
        );
        let mut method_tag_types = match shared {
            Some(_) => std::borrow::Cow::Borrowed(tag_types_by_name.as_ref()),
            None => std::borrow::Cow::Owned(tag_types_by_name.as_ref().clone()),
        };
        if let Some(receiver_type) = &receiver_type {
            method_tag_types
                .to_mut()
                .insert(Intern::<String>::from_ref("Self"), receiver_type.clone());
        }
        let mut return_ty = resolve_return_type(
            declare,
            &typed.type_registry,
            &method_tag_types,
            &tag_params,
            (&file_ast.tags).into(),
            dependent_binder,
            &method_typevar_subst,
            receiver_type.as_ref(),
            declare
                .receiver_type_surface()
                .map(|receiver| &receiver.value),
            &result_family_owner,
        );
        if return_ty == Ty::Unit
            && declare.is_constant()
            && target_dependent_names
                .as_ref()
                .is_some_and(|names| names.contains(&name.0))
            && let BindValue::Expr(expr) = &declare.value
        {
            let inferred = expr.infer_ty(&TyInferEnv {
                tag_types: &method_tag_types,
                fn_return_types: &HashMap::new(),
                locals: &HashMap::new(),
                tag_params: Some(&tag_params),
            });
            if inferred.is_int()
                || matches!(
                    inferred,
                    Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
                )
            {
                return_ty = Ty::AnonymousInteger {
                    validity: ast::integer::IntegerValidity::new(IntegerDomain::symbolic(
                        ast::NormalExpr::Var(name.0),
                    )),
                };
            }
        }
        let param_types = resolve_param_types(
            declare,
            &typed.type_registry,
            &method_tag_types,
            &tag_params,
            (&file_ast.tags).into(),
            dependent_binder,
            &method_typevar_subst,
        );

        let param_kinds: Vec<ParamKind> = declare
            .params
            .as_ref()
            .map(|params| {
                params
                    .iter()
                    .zip(&param_types)
                    .map(|((name, parameter), (_, ty))| {
                        if declare.param_refinements.contains_key(name)
                            || super::parameters::is_inferred_value(declare, *name)
                            || (name.as_str() == "self" && receiver_type.is_some())
                            || matches!(&parameter.kind, ParameterKind::Tagged(surface) if matches!(surface.value, Expr::IntegerDomain(_)))
                        {
                            ParamKind::Value(Box::new(ty.clone()))
                        } else {
                            match param_kind_from_parameter_kind(&parameter.kind, &tag_types_by_name) {
                                ParamKind::Value(_) => ParamKind::Value(Box::new(ty.clone())),
                                kind => kind,
                            }
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let param_conventions: Vec<ParamConvention> = declare
            .params
            .as_ref()
            .map(|params| {
                params
                    .keys()
                    .map(|name| {
                        declare
                            .param_conventions
                            .get(name)
                            .copied()
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let param_refinements: Vec<_> = declare
            .params
            .as_ref()
            .map(|params| {
                params
                    .keys()
                    .map(|name| declare.param_refinements.get(name).cloned())
                    .collect()
            })
            .unwrap_or_default();
        let (param_groups, groups, result_groups) = resolve_target_groups(
            declare,
            &param_types,
            &param_conventions,
            &typed.type_registry,
            &mut typed.declaration_flaws,
        );
        let param_count = param_types.len();

        let mut bind_flaws = Vec::new();
        if declare.attributes.is_private() {
            if receiver_type.is_none() {
                // type-private-member-context
                bind_flaws.push(flaw(
                    &typed.span_table,
                    declare.name_span,
                    DiagnosticCode::Type(TypeCode::Member(MemberCode::PrivateContext)),
                    "#private belongs on a field or method",
                ));
            } else {
                typed.type_registry.insert_private_method(
                    def_id,
                    crate::type_registry::PrivateMethod {
                        origin: file_ast.semantic_origin.clone(),
                        file: typed.file_id.0,
                        grants: private_grants(&declare.attributes),
                    },
                );
            }
        }

        for (span, diagnostic) in super::parameters::constraint_diagnostics(
            declare,
            &file_ast.tags,
            &ctx.compile_time_eval_ast.tags,
        ) {
            bind_flaws.push(diagnostic.at_span_id(span, &typed.span_table));
        }
        let domain_types = TypeEnv::new(&method_tag_types)
            .with_registry(&typed.type_registry)
            .with_tag_params(&tag_params)
            .with_tag_decls(&file_ast.tags)
            .with_dependent_binder(dependent_binder);
        let param_constraints = super::parameters::resolve_constraints(declare, &domain_types);
        for (span, diagnostic) in argument_context.bind_diagnostics(declare) {
            bind_flaws.push(diagnostic.at_span_id(span, &typed.span_table));
        }
        for surface in declare
            .params
            .iter()
            .flat_map(|params| params.values())
            .filter_map(|param| match &param.kind {
                ParameterKind::Tagged(surface) => Some(surface.as_ref()),
                _ => None,
            })
            .chain(declare.return_tag.as_deref())
        {
            if let Some(diagnostic) =
                domain_types.integer_domain_diagnostic(surface, &typed.span_table)
            {
                bind_flaws.push(diagnostic);
            }
        }

        if param_types
            .iter()
            .any(|(_, ty)| contains_unresolved_result_label(ty, &anonymous_result_labels))
            || contains_unresolved_result_label(&return_ty, &anonymous_result_labels)
        {
            bind_flaws.push(anonymous_result_annotation_diagnostic(
                declare.name_span,
                &typed.span_table,
            ));
        }
        validate_result_proof_surfaces(
            declare,
            &ctx.compile_time_eval_ast,
            &mut bind_flaws,
            &typed.span_table,
        );
        if let Some(symbol) = declare.attributes.infix_symbol
            && ast::BinOp::from_symbol(symbol.as_str()).is_none()
        {
            bind_flaws.push(
                // type-unknown-infix-operator
                Diagnostic::new(
                    OperatorCode::UnknownInfix,
                    format!("unknown infix operator `{}`", symbol.as_str()),
                )
                .with_arg("operator", symbol.as_str().to_string())
                .at_span_id(
                    declare
                        .attributes
                        .infix_symbol_span
                        .unwrap_or(declare.name_span),
                    &typed.span_table,
                ),
            );
        }
        if let Some(symbol) = declare.attributes.infix_symbol
            && let Some(op) = ast::BinOp::from_symbol(symbol.as_str())
            && ast::ProofRelation::for_bin_op(&op).is_some()
        {
            // type-fixed-numeric-comparison
            bind_flaws.push(flaw(
                &typed.span_table,
                declare.name_span,
                DiagnosticCode::Type(TypeCode::Operator(OperatorCode::FixedNumericComparison)),
                "numeric comparison operators have fixed semantics; use a named comparison function for custom orderings",
            ));
        }
        let value_param_count = param_kinds
            .iter()
            .filter(|kind| matches!(kind, ParamKind::Value(_)))
            .count();
        if declare.attributes.infix_symbol.is_some() && value_param_count != 2 {
            // type-invalid-infix-declaration
            bind_flaws.push(flaw(
                &typed.span_table,
                declare.name_span,
                DiagnosticCode::Type(TypeCode::Operator(OperatorCode::InvalidInfixDeclaration)),
                "an infix declaration must have exactly two value parameters",
            ));
        }
        if matches!(
            declare
                .attributes
                .infix_symbol
                .as_ref()
                .map(|symbol| symbol.as_str()),
            Some("+") | Some("-") | Some("*") | Some("&") | Some("|") | Some("^")
        ) && param_types.first().is_some_and(|(_, operand)| {
            typed
                .type_registry
                .integer_validity_for_type(operand.without_reference_wrappers())
                .is_some()
                && operand != &return_ty
                && !(matches!(
                    declare
                        .attributes
                        .infix_symbol
                        .as_deref()
                        .map(String::as_str),
                    Some("+" | "-" | "*")
                ) && checked_integer_operator_result(
                    &typed.type_registry,
                    operand,
                    &return_ty,
                ))
        }) {
            // type-invalid-operator-result
            bind_flaws.push(flaw(
                &typed.span_table,
                declare.name_span,
                DiagnosticCode::Type(TypeCode::Operator(OperatorCode::InvalidResult)),
                "an integer operator must return its left operand type; arithmetic may also return a union of that type and Flaw",
            ));
        }
        for contract in trait_methods.contracts(declare) {
            let mut expected = declare.clone();
            expected.params = Some(contract.method.params.as_ref().clone());
            expected.param_conventions = contract.method.conventions.as_ref().clone();
            expected.param_refinements = contract.method.param_refinements.as_ref().clone();
            expected.return_tag = contract.method.return_ty.clone();
            expected.anonymous_result_alternatives =
                contract.method.anonymous_result_alternatives.clone();
            let expected_params = resolve_param_types(
                &expected,
                &typed.type_registry,
                &method_tag_types,
                &tag_params,
                (&file_ast.tags).into(),
                dependent_binder,
                &method_typevar_subst,
            );
            let expected_return = resolve_return_type(
                &expected,
                &typed.type_registry,
                &method_tag_types,
                &tag_params,
                (&file_ast.tags).into(),
                dependent_binder,
                &method_typevar_subst,
                receiver_type.as_ref(),
                declare.receiver_type_surface().map(|r| &r.value),
                &result_family_owner,
            );
            let expected_conventions: Vec<_> = contract
                .method
                .params
                .keys()
                .map(|name| {
                    contract
                        .method
                        .conventions
                        .get(name)
                        .copied()
                        .unwrap_or_default()
                })
                .collect();
            if !param_types
                .iter()
                .map(|(_, ty)| ty)
                .eq(expected_params.iter().map(|(_, ty)| ty))
                || param_conventions != expected_conventions
                || param_constraints
                    != super::parameters::resolve_constraints(&expected, &domain_types)
                || !return_contract_matches(&typed.type_registry, &return_ty, &expected_return)
                || contract
                    .method
                    .attributes
                    .infix_symbol
                    .is_some_and(|s| Some(s) != declare.attributes.infix_symbol)
            {
                // type-trait-method-mismatch
                bind_flaws.push(Diagnostic::new(
                    TraitCode::MethodMismatch,
                    format!("implementation does not match `{}.{}`; use a qualified method name for conflicting contracts", contract.owner, contract.method.name),
                ).with_arg("trait_name", contract.owner.as_str().to_string())
                 .with_arg("method_name", contract.method.name.as_str().to_string())
                 .at_span_id(declare.name_span, &typed.span_table));
            }
        }
        let typed_bind = TypedBind {
            name: name.0,
            dependent_binder,
            name_span: declare.name_span,
            terminal_exit_span: match &declare.value {
                ast::BindValue::Body { ret, .. } => ret.span_id,
                ast::BindValue::Expr(expr) => {
                    let end = typed.span_table.get(expr.span_id).end();
                    typed.span_table.insert(ast::Span::new(end, end))
                }
                ast::BindValue::Unassigned => declare.name_span,
            },
            return_type: return_ty.clone(),
            named_results: declare.named_results.clone().into(),
            declared_return_type: std::sync::Arc::new(return_ty.clone()),
            return_evidence: None,
            postcondition: None,
            params: param_types.into(),
            param_kinds: param_kinds.into(),
            param_constraints: param_constraints.into(),
            param_conventions: param_conventions.into(),
            param_groups: param_groups.into(),
            param_refinements: param_refinements.into(),
            payload_postconditions: declare
                .payload_postconditions
                .iter()
                .map(|proof| {
                    let is_self = matches!(
                        &proof.ty.value,
                        Expr::AnonymousTag(name) if name.as_str() == "Self"
                    );
                    let ty = if is_self {
                        receiver_type
                            .clone()
                            .unwrap_or(Ty::Opaque(Intern::from_ref("Self")))
                    } else {
                        TypeEnv::new(&method_tag_types)
                            .with_registry(&typed.type_registry)
                            .with_subst(&method_typevar_subst)
                            .with_tag_params(&tag_params)
                            .with_tag_declaration_lookup((&file_ast.tags).into())
                            .with_dependent_binder(dependent_binder)
                            .resolve_expr(&proof.ty.value)
                    };
                    crate::typed::TypedPayloadPostcondition {
                        name: proof.name,
                        ty,
                        proposition: proof.proposition.clone(),
                    }
                })
                .collect::<Vec<_>>()
                .into(),
            groups: groups.into(),
            result_groups: result_groups.into(),
            references: Default::default(),
            effects: Default::default(),
            body_type_parameters: std::sync::Arc::new(receiver_body_type_parameters(
                declare,
                receiver_type.as_ref(),
            )),
            receiver_type,
            unassigned_decl: matches!(declare.value, BindValue::Unassigned),
            is_callable: declare.params.is_some(),
            is_constant: declare.is_constant(),
            body: crate::typed::BindBody::Unlowered,
            flaws: std::sync::Arc::new(bind_flaws),
            param_requirements: smallvec::smallvec![AvailabilityRequirement::Any; param_count],
            call_capability: CallCapability::StagePolymorphic,
            definition_stage: crate::typed::DefinitionStage::StagePolymorphic,
            signature_surface: declare.signature_surface().into(),
            doc_comment: declare.doc_comment.clone().map(std::sync::Arc::new),
            attributes: std::sync::Arc::new(declare.attributes.clone()),
            infix_symbol: declare.attributes.infix_symbol,
        };
        if declare.params.is_some() {
            typed.fn_return_types.insert(def_id, return_ty.clone());
        }
        typed.defs.insert(def_id, typed_bind);
    }

    for declare in file_ast.tags.values() {
        if let ast::DeclareValue::Has(members) = &declare.value {
            for member in members {
                let ast::HasMember::Property(field) = member else {
                    continue;
                };
                if field.owned_storage && (field.body.is_some() || field.qualifier.is_some()) {
                    typed.declaration_flaws.push((
                        field.name_span,
                        // type-invalid-owned-storage-field
                        Diagnostic::new(
                            OwnershipCode::StorageFieldInvalid,
                            "own storage must be a stored field",
                        )
                        .at_span_id(field.name_span, &typed.span_table),
                    ));
                }
            }
        }
        for attribute in declare.attributes.raw_attributes.iter().flatten() {
            if matches!(attribute, ast::AttributeItem::Flag { name, .. } | ast::AttributeItem::Call { name, .. } if name.as_str() == "private")
            {
                typed.declaration_flaws.push((
                    declare.name_span,
                    // type-private-member-context
                    Diagnostic::new(
                        MemberCode::PrivateContext,
                        "#private belongs on a field or method",
                    )
                    .at_span_id(declare.name_span, &typed.span_table),
                ));
            }
        }
        if let ast::DeclareValue::Has(members) = &declare.value {
            for member in members {
                let (span, attributes) = match member {
                    ast::HasMember::Property(field) => (field.name_span, &field.attributes),
                    ast::HasMember::Function(method) => (method.name_span, &method.attributes),
                };
                for attribute in attributes.raw_attributes.iter().flatten() {
                    if let ast::AttributeItem::Call { name, args, .. } = attribute
                        && name.as_str() == "private"
                        && (args.is_empty() || private_grants(attributes).len() != args.len())
                    {
                        typed.declaration_flaws.push((
                            span,
                            // type-invalid-private-grant
                            Diagnostic::new(
                                MemberCode::PrivateGrantInvalid,
                                "private grants require module names",
                            )
                            .at_span_id(span, &typed.span_table),
                        ));
                    }
                }
            }
        }
    }

    // 5. Build type scope for declarations.
    let scope_names = build_type_scope(file_ast, ctx);
    let type_scope: HashSet<Intern<String>> = scope_names.into_iter().collect();

    for declare in file_ast.tags.values() {
        for provided in &declare.provided_traits {
            if !type_scope.contains(&provided.trait_name) {
                // type-unknown-symbol
                let diagnostic = Diagnostic::new(
                    SymbolCode::Unknown,
                    format!("unknown symbol `{}`", provided.trait_name.as_str()),
                )
                .with_arg("name", provided.trait_name.as_str().to_string())
                .at_span_id(provided.trait_name_span, &typed.span_table);
                typed
                    .declaration_flaws
                    .push((provided.trait_name_span, diagnostic));
            }
        }
        check_declare_value_type_scope(&declare.value, &type_scope, &mut |span, symptom| {
            let symptom = symptom.at_span_id(span, &typed.span_table);
            typed.declaration_flaws.push((span, symptom))
        });
        if let Some(parameters) = &declare.params {
            let phantom_parameters = declare.attributes.phantom_parameters();
            for phantom in &phantom_parameters {
                if !parameters.contains_key(phantom) {
                    typed.declaration_flaws.push((
                        declare.name_span,
                        // type-unknown-phantom-parameter
                        Diagnostic::new(
                            GenericCode::UnknownPhantomParameter,
                            format!("unknown phantom parameter `{}`", phantom.as_str()),
                        )
                        .with_arg("parameter", phantom.as_str().to_string())
                        .at_span_id(declare.name_span, &typed.span_table),
                    ));
                }
            }
            if matches!(
                declare.value,
                DeclareValue::Set() | DeclareValue::Range(_, _) | DeclareValue::InRange(_, _)
            ) {
                for parameter in parameters
                    .keys()
                    .filter(|parameter| !phantom_parameters.contains(parameter))
                {
                    typed.declaration_flaws.push((
                        declare.name_span,
                        // type-unused-generic-parameter
                        Diagnostic::new(
                            GenericCode::UnusedParameter,
                            format!(
                                "generic parameter `{}` does not participate in `{}`",
                                parameter.as_str(),
                                declare.name.as_str()
                            ),
                        )
                        .with_arg("parameter", parameter.as_str().to_string())
                        .with_arg("type_name", declare.name.as_str().to_string())
                        .with_help(format!(
                            "mark it `#phantom({})` if it intentionally distinguishes type identity",
                            parameter.as_str()
                        ))
                        .at_span_id(declare.name_span, &typed.span_table),
                    ));
                }
            }
        }
    }

    for bind in typed.defs.values_mut() {
        let unsaturated = std::iter::once(&bind.return_type)
            .chain(bind.params.iter().map(|(_, ty)| ty))
            .find_map(|ty| unsaturated_type_name(ty, &typed.type_registry));
        if let Some(name) = unsaturated {
            bind.push_flaw(
                // type-unsaturated-application
                Diagnostic::new(
                    GenericCode::UnsaturatedApplication,
                    format!("generic type `{}` requires type arguments", name.as_str()),
                )
                .with_arg("type_name", name.as_str().to_string())
                .at_span_id(bind.name_span, &typed.span_table),
            );
        }
    }

    for (name, bind) in &file_ast.defs {
        let Some(surface) = bind.return_tag.as_deref() else {
            continue;
        };
        let Some(type_name) = unsaturated_type_surface(&surface.value, &tag_params) else {
            continue;
        };
        if let Some(typed_bind) = typed.defs.get_mut(name) {
            typed_bind.push_flaw(
                // type-unsaturated-application
                Diagnostic::new(
                    GenericCode::UnsaturatedApplication,
                    format!(
                        "generic type `{}` requires type arguments",
                        type_name.as_str()
                    ),
                )
                .with_arg("type_name", type_name.as_str().to_string())
                .at_span_id(surface.span_id, &typed.span_table),
            );
        }
    }

    for (name, declare) in &file_ast.tags {
        for (span, diagnostic) in argument_context.declaration_diagnostics(declare) {
            typed
                .declaration_flaws
                .push((span, diagnostic.at_span_id(span, &typed.span_table)));
        }
        let unsaturated = match &declare.value {
            DeclareValue::Alias(surface) => unsaturated_type_surface(&surface.value, &tag_params),
            DeclareValue::Has(members) => members.iter().find_map(|member| match member {
                ast::HasMember::Property(property) => property
                    .ty
                    .as_ref()
                    .and_then(|surface| unsaturated_type_surface(&surface.value, &tag_params)),
                ast::HasMember::Function(_) => None,
            }),
            _ => None,
        };
        if let Some(type_name) = unsaturated {
            let span = typed
                .tags
                .get(&TagId(*name))
                .map(|tag| tag.name_span)
                .unwrap_or(declare.name_span);
            typed.declaration_flaws.push((
                span,
                // type-unsaturated-application
                Diagnostic::new(
                    GenericCode::UnsaturatedApplication,
                    format!(
                        "generic type `{}` requires type arguments",
                        type_name.as_str()
                    ),
                )
                .with_arg("type_name", type_name.as_str().to_string())
                .at_span_id(span, &typed.span_table),
            ));
        }
    }

    for (tag_id, tag) in &*typed.tags {
        let is_own_nominal_root = tag
            .resolved_ty
            .named_instance()
            .and_then(|instance| typed.type_registry.declaration_for_instance(instance))
            .is_some_and(|declaration| declaration.display_name == tag_id.0);
        let subject = if is_own_nominal_root {
            typed
                .type_registry
                .resolved_definition_for_type(&tag.resolved_ty)
        } else {
            tag.resolved_ty.clone()
        };
        if let Some(name) = unsaturated_type_name(&subject, &typed.type_registry) {
            typed.declaration_flaws.push((
                tag.name_span,
                // type-unsaturated-application
                Diagnostic::new(
                    GenericCode::UnsaturatedApplication,
                    format!("generic type `{}` requires type arguments", name.as_str()),
                )
                .with_arg("type_name", name.as_str().to_string())
                .at_span_id(tag.name_span, &typed.span_table),
            ));
        }
    }

    crate::typed::sort_variant_map(&mut typed.variant_map);
    typed
}

fn unsaturated_type_name(
    ty: &Ty,
    registry: &crate::type_registry::TypeRegistry,
) -> Option<Intern<String>> {
    match ty {
        Ty::Named { instance, .. } => {
            let declaration = registry.declaration_for_instance(instance)?;
            if instance.arguments.len() < declaration.parameters.len() {
                return Some(declaration.display_name);
            }
            instance
                .arguments
                .iter()
                .find_map(|(_, argument)| match argument {
                    ast::TyArg::Type(ty) => unsaturated_type_name(ty, registry),
                    ast::TyArg::Const(_) => None,
                })
        }
        Ty::Record { fields, .. } => fields
            .iter()
            .find_map(|(_, field)| unsaturated_type_name(field, registry)),
        Ty::Union { variants, .. } => variants.iter().find_map(|variant| {
            variant
                .fields
                .iter()
                .find_map(|(_, field)| unsaturated_type_name(field, registry))
                .or_else(|| {
                    variant
                        .result_ty
                        .as_ref()
                        .and_then(|ty| unsaturated_type_name(ty, registry))
                })
        }),
        Ty::Array { elem, .. }
        | Ty::Ptr { inner: elem }
        | Ty::OwnedStorage { inner: elem }
        | Ty::Address { pointee: elem, .. }
        | Ty::Ref { inner: elem, .. } => unsaturated_type_name(elem, registry),
        Ty::UnresolvedSequence { element, .. } => element
            .as_deref()
            .and_then(|element| unsaturated_type_name(element, registry)),
        Ty::Sequence(fields) => fields
            .iter()
            .find_map(|field| unsaturated_type_name(&field.ty, registry)),
        Ty::Proposition(_)
        | Ty::ResultFamily { .. }
        | Ty::AnonymousInteger { .. }
        | Ty::UnresolvedLiteral(_)
        | Ty::Float { .. }
        | Ty::Unit
        | Ty::Opaque(_)
        | Ty::Literal(_) => None,
    }
}

fn unsaturated_type_surface(
    expression: &Expr,
    tag_params: &HashMap<Intern<String>, Parameters>,
) -> Option<Intern<String>> {
    match expression {
        Expr::AnonymousTag(name) => tag_params
            .get(name)
            .is_some_and(|parameters| {
                parameters.values().any(|parameter| {
                    !matches!(
                        parameter.kind,
                        ParameterKind::Default(_) | ParameterKind::Inferred { .. }
                    )
                })
            })
            .then_some(*name),
        Expr::TagCall(call) => tag_params
            .get(&call.name)
            .filter(|parameters| {
                let required = parameters
                    .values()
                    .take_while(|parameter| {
                        !matches!(
                            parameter.kind,
                            ParameterKind::Default(_) | ParameterKind::Inferred { .. }
                        )
                    })
                    .count();
                call.args.len() < required || call.args.len() > parameters.len()
            })
            .map(|_| call.name)
            .or_else(|| {
                call.args
                    .iter()
                    .find_map(|argument| unsaturated_type_surface(&argument.value, tag_params))
            }),
        Expr::TakePtr(inner) | Expr::Ref { inner, .. } => {
            unsaturated_type_surface(&inner.value, tag_params)
        }
        _ => None,
    }
}

fn declaration_recursively_refers_to_name(
    declaration_name: &Intern<String>,
    declarations: &ast::TagMap,
    resolved_tags: &HashMap<Intern<String>, Ty>,
    recursion_stack: &mut HashSet<Intern<String>>,
) -> bool {
    if declarations.get(declaration_name).is_none() {
        return false;
    }
    if !recursion_stack.insert(*declaration_name) {
        return true;
    }
    let Some(root) = resolved_tags.get(declaration_name) else {
        recursion_stack.remove(declaration_name);
        return false;
    };
    let has_cycle =
        declaration_ty_refers_to_name(root, declarations, resolved_tags, recursion_stack);
    recursion_stack.remove(declaration_name);
    has_cycle
}

fn declaration_ty_refers_to_name(
    ty: &Ty,
    declarations: &ast::TagMap,
    resolved_tags: &HashMap<Intern<String>, Ty>,
    recursion_stack: &mut HashSet<Intern<String>>,
) -> bool {
    let name = match ty {
        Ty::Ref { .. } | Ty::Ptr { .. } | Ty::OwnedStorage { .. } | Ty::Address { .. } => {
            return false;
        }
        Ty::Named { instance, .. } => &instance.declaration.name,
        Ty::Opaque(name) => name,
        _ => {
            return ty.any_child(|child| {
                child.ty().is_some_and(|ty| {
                    declaration_ty_refers_to_name(ty, declarations, resolved_tags, recursion_stack)
                })
            });
        }
    };
    declarations.contains_key(name)
        && declaration_recursively_refers_to_name(
            name,
            declarations,
            resolved_tags,
            recursion_stack,
        )
}

fn declaration_reference_semantics(
    ty: &Ty,
    type_registry: &crate::type_registry::TypeRegistry,
) -> Result<Option<ReferenceSemantics>, Box<Diagnostic>> {
    let Some(reference) = reference_semantics_for_type(ty, Some(type_registry)) else {
        return Ok(None);
    };

    if is_nested_reference_former(ty, Some(type_registry)) {
        // type-invalid-reference-former
        return Err(Box::new(Diagnostic::new(
            ReferenceCode::InvalidFormer,
            "reference former declarations cannot be nested",
        )));
    }

    Ok(Some(reference))
}

fn receiver_body_type_parameters(
    bind: &ast::Bind,
    receiver: Option<&Ty>,
) -> BTreeSet<Intern<String>> {
    let candidates: BTreeSet<_> = receiver
        .and_then(Ty::named_instance)
        .into_iter()
        .flat_map(|instance| &instance.arguments)
        .filter_map(|(_, argument)| match argument {
            ast::TyArg::Type(ty) => match ty.as_ref() {
                Ty::Opaque(name) => Some(*name),
                _ => None,
            },
            _ => None,
        })
        .collect();
    struct Uses<'a> {
        candidates: &'a BTreeSet<Intern<String>>,
        names: BTreeSet<Intern<String>>,
    }
    impl Uses<'_> {
        fn name(&mut self, name: Intern<String>) {
            if name.as_str() == "Self" {
                self.names.extend(self.candidates.iter().copied());
            } else if self.candidates.contains(&name) {
                self.names.insert(name);
            }
        }
    }
    impl ProofVisitor for Uses<'_> {
        fn visit_reference(&mut self, ty: &ast::TypeReference) -> ControlFlow<()> {
            if let Some(name) = ty.head_name() {
                self.name(name);
            }
            walk_reference(self, ty)
        }
    }
    impl ast::folder::Visitor for Uses<'_> {
        fn visit_expr(&mut self, expr: &ast::Expr) -> std::ops::ControlFlow<()> {
            match expr {
                Expr::AnonymousTag(name) => self.name(*name),
                Expr::FnCall(call) => self.name(call.path.root),
                Expr::TagCall(call) => self.name(call.name),
                Expr::Cast { ty, .. } | Expr::TargetQuery { operand: ty, .. } => {
                    let _ = self.visit_reference(ty);
                }
                _ => {}
            }
            ast::folder::walk_expr(self, expr)
        }
    }
    use ast::folder::Visitor;
    let mut uses = Uses {
        candidates: &candidates,
        names: BTreeSet::new(),
    };
    let _ = uses.visit_bind_value(&bind.value);
    uses.names
}

pub(super) fn validate_declaration_representations(
    typed: &mut TypedFileAst,
    file_ast: &FileAst,
    ctx: &TransformCtx,
) {
    let _profile = compiler_profile::span(compiler_profile::Phase::Representations);
    let target_dependent_names = matches!(ctx.compile_target, flask::CompileTarget::Library)
        .then(|| crate::prepare_target::target_dependent_bind_names(&ctx.compile_time_eval_ast));
    for (name, declare) in &file_ast.tags {
        let Some(tag) = typed.tags.get(&TagId(*name)) else {
            continue;
        };
        let resolved_ty = &tag.resolved_ty;
        if resolved_ty.named_instance().is_some() {
            let has_generic_parameters = declare.params.as_ref().is_some_and(|params| {
                params
                    .values()
                    .any(|parameter| is_type_parameter(&parameter.kind))
            });
            if !has_generic_parameters
                && !target_dependent_names
                    .as_ref()
                    .is_some_and(|names| declaration_depends_on_target(declare, names))
                && !crate::representation::has_symbolic_fixed_array_length(
                    resolved_ty,
                    Some(&typed.type_registry),
                )
            {
                match crate::representation::Repr::derive(resolved_ty, Some(&typed.type_registry)) {
                    Ok(_) | Err(RepresentationError::TargetDependentInteger { .. }) => {}
                    Err(error) => {
                        let diagnostic = error
                            .diagnostic()
                            .with_arg("declaration", declare.name.as_str().to_string())
                            .with_arg("type_name", declare.name.as_str().to_string())
                            .at_span_id(declare.name_span, &typed.span_table);
                        typed
                            .declaration_flaws
                            .push((declare.name_span, diagnostic));
                    }
                }
            }
        }
    }
}

fn is_nested_reference_former(
    ty: &Ty,
    type_registry: Option<&crate::type_registry::TypeRegistry>,
) -> bool {
    match ty {
        Ty::Ref { inner, .. } => match inner.as_ref() {
            Ty::Ref { .. } => true,
            ty => type_registry.is_some_and(|type_registry| {
                reference_semantics_for_type(ty, Some(type_registry)).is_some()
            }),
        },
        _ => type_registry.is_some_and(|type_registry| {
            reference_semantics_for_type(ty, Some(type_registry)).is_some()
        }),
    }
}

fn declaration_instance_arguments(
    declare: &ast::declare::Declare,
) -> Vec<(Intern<String>, ast::TyArg)> {
    let Some(params) = &declare.params else {
        return Vec::new();
    };
    params
        .iter()
        .map(|(name, parameter)| {
            let argument = if is_type_parameter(&parameter.kind) {
                ast::TyArg::Type(Box::new(Ty::Opaque(*name)))
            } else {
                ast::TyArg::Const(ast::NormalExpr::Var(*name))
            };
            (*name, argument)
        })
        .collect()
}

fn preserve_resolved_params(
    params: &[(Intern<String>, ast::TyArg)],
    declaration_name: Intern<String>,
    instance: &ast::ty::NamedTypeInstance,
) -> Vec<(Intern<String>, ast::TyArg)> {
    params
        .iter()
        .map(|(name, arg)| {
            let arg = match arg {
                ast::TyArg::Type(ty) => ast::TyArg::Type(Box::new(
                    preserve_self_reference_with_instance(ty, declaration_name, instance),
                )),
                ast::TyArg::Const(expr) => ast::TyArg::Const(expr.clone()),
            };
            (*name, arg)
        })
        .collect()
}

fn preserve_self_reference_with_instance(
    ty: &Ty,
    declaration_name: Intern<String>,
    instance: &ast::ty::NamedTypeInstance,
) -> Ty {
    match ty {
        Ty::Named {
            instance: ty_instance,
            name,
            ..
        } if *name == declaration_name && ty_instance.declaration == instance.declaration => {
            Ty::Named {
                instance: ty_instance.clone(),
                name: *name,
            }
        }
        Ty::Named {
            instance: ty_instance,
            name,
        } => Ty::Named {
            instance: ty_instance.clone().with_arguments(
                ty_instance
                    .arguments
                    .iter()
                    .map(|(parameter, argument)| {
                        let argument = match argument {
                            ast::TyArg::Type(ty) => {
                                ast::TyArg::Type(Box::new(preserve_self_reference_with_instance(
                                    ty,
                                    declaration_name,
                                    instance,
                                )))
                            }
                            ast::TyArg::Const(expr) => ast::TyArg::Const(expr.clone()),
                        };
                        (*parameter, argument)
                    })
                    .collect(),
            ),
            name: *name,
        },
        Ty::Ref { inner, mutable } => Ty::Ref {
            inner: Box::new(preserve_self_reference_with_instance(
                inner,
                declaration_name,
                instance,
            )),
            mutable: *mutable,
        },
        Ty::Record {
            name,
            fields,
            resolved_params,
        } => Ty::Record {
            name: *name,
            fields: fields
                .iter()
                .map(|(field_name, field_ty)| {
                    (
                        *field_name,
                        Box::new(preserve_self_reference_with_instance(
                            field_ty,
                            declaration_name,
                            instance,
                        )),
                    )
                })
                .collect(),
            resolved_params: resolved_params
                .as_ref()
                .map(|params| preserve_resolved_params(params, declaration_name, instance)),
        },
        Ty::Union {
            name,
            variants,
            literal_values,
            resolved_params,
        } => Ty::Union {
            name: *name,
            variants: variants
                .iter()
                .map(|variant| UnionVariant {
                    kind: variant.kind,
                    discriminant: variant.discriminant,
                    name: variant.name,
                    fields: variant
                        .fields
                        .iter()
                        .map(|(name, field_ty)| {
                            (
                                *name,
                                Box::new(preserve_self_reference_with_instance(
                                    field_ty,
                                    declaration_name,
                                    instance,
                                )),
                            )
                        })
                        .collect(),
                    result_ty: variant.result_ty.as_ref().map(|result_ty| {
                        preserve_self_reference_with_instance(result_ty, declaration_name, instance)
                    }),
                })
                .collect(),
            literal_values: literal_values.clone(),
            resolved_params: resolved_params
                .as_ref()
                .map(|params| preserve_resolved_params(params, declaration_name, instance)),
        },
        Ty::Opaque(name) if *name == declaration_name => Ty::Named {
            instance: instance.clone(),
            name: *name,
        },
        Ty::Array { elem, size } => Ty::Array {
            elem: Box::new(preserve_self_reference_with_instance(
                elem,
                declaration_name,
                instance,
            )),
            size: size.clone(),
        },
        Ty::UnresolvedSequence { element, length } => Ty::UnresolvedSequence {
            element: element.as_deref().map(|element| {
                Box::new(preserve_self_reference_with_instance(
                    element,
                    declaration_name,
                    instance,
                ))
            }),
            length: length.clone(),
        },
        Ty::OwnedStorage { inner } => Ty::OwnedStorage {
            inner: Box::new(preserve_self_reference_with_instance(
                inner,
                declaration_name,
                instance,
            )),
        },
        Ty::Ptr { inner } => Ty::Ptr {
            inner: Box::new(preserve_self_reference_with_instance(
                inner,
                declaration_name,
                instance,
            )),
        },
        Ty::Address {
            pointee,
            address_space,
        } => Ty::Address {
            pointee: Box::new(preserve_self_reference_with_instance(
                pointee,
                declaration_name,
                instance,
            )),
            address_space: *address_space,
        },
        Ty::Sequence(elements) => Ty::Sequence(
            elements
                .iter()
                .map(|element| ast::ty::SequenceTypeElement {
                    name: element.name,
                    ty: Box::new(preserve_self_reference_with_instance(
                        &element.ty,
                        declaration_name,
                        instance,
                    )),
                })
                .collect(),
        ),
        Ty::Literal(v) => Ty::Literal(v.clone()),
        Ty::AnonymousInteger { .. }
        | Ty::Proposition(_)
        | Ty::ResultFamily { .. }
        | Ty::UnresolvedLiteral(_)
        | Ty::Float { .. }
        | Ty::Unit => ty.clone(),
        Ty::Opaque(name) => Ty::Opaque(*name),
    }
}

fn build_type_scope(file_ast: &FileAst, _ctx: &TransformCtx) -> HashSet<Intern<String>> {
    let mut names: HashSet<Intern<String>> = HashSet::new();
    names.extend(file_ast.tags.keys().copied());
    names.extend(file_ast.defs.keys().map(|id| id.0));
    for import in &file_ast.uses {
        collect_import_scope_names(import, &mut names);
    }
    for alias in &file_ast.symbol_aliases {
        names.insert(alias.alias);
    }
    names
}

fn collect_import_scope_names(mi: &Import, names: &mut HashSet<Intern<String>>) {
    for m in &mi.0 {
        match &m.source {
            ImportSource::CurrentModule { member } => {
                names.insert(member.alias.unwrap_or(member.export));
            }
            ImportSource::LocalBundle(b) => {
                for m in &b.members {
                    names.insert(m.flat_name());
                }
            }
            ImportSource::Package(_)
            | ImportSource::Local(_, _)
            | ImportSource::LocalFile(_, _) => {
                if let Some(alias) = m.alias {
                    names.insert(alias);
                } else {
                    names.insert(Intern::new(m.effective_name()));
                }
            }
        }
    }
}

fn check_expr_value_scope(
    expr: &Typed<Expr>,
    type_scope: &HashSet<Intern<String>>,
    push_flaw: &mut dyn FnMut(SpanId, Diagnostic),
) {
    match &expr.value {
        Expr::RecordGet { base, .. } => check_expr_value_scope(base, type_scope, push_flaw),
        Expr::FnCall(call) if call.path.value.segments.is_empty() => {
            let name = call.path.value.root;
            if !type_scope.contains(&name) {
                push_flaw(
                    expr.span_id,
                    // type-unknown-symbol
                    Diagnostic::new(
                        SymbolCode::Unknown,
                        format!("unknown symbol `{}`", name.as_str()),
                    )
                    .with_arg("name", name.as_str().to_string()),
                );
            }
        }
        _ => {}
    }
}

fn check_union_variant_shape_type_scope(
    declare_value: &DeclareValue,
    type_scope: &HashSet<Intern<String>>,
    push_flaw: &mut dyn FnMut(SpanId, Diagnostic),
) {
    match declare_value {
        DeclareValue::Union { variants } => {
            for variant in variants {
                let shape = variant.shape();
                let params = match &shape.value {
                    Pattern::Generic { params, .. } => Some(params),
                    _ => None,
                };
                if let Some(params) = params {
                    for (_, kind) in params {
                        if let ast::parameter::ParameterKind::Tagged(sp) = kind
                            && let Expr::AnonymousTag(name) = &sp.value
                            && name.as_str().is_capitalized_type_name()
                            && !type_scope.contains(name)
                        {
                            push_flaw(
                                sp.span_id,
                                // type-unknown-symbol
                                Diagnostic::new(
                                    SymbolCode::Unknown,
                                    format!("unknown symbol `{}`", name.as_str()),
                                )
                                .with_arg("name", name.as_str().to_string()),
                            );
                        }
                    }
                }
            }
        }
        DeclareValue::When(w) => {
            if let Some(subject) = &w.subject {
                check_expr_value_scope(subject, type_scope, push_flaw);
            }
            for arm in &w.arms {
                match arm {
                    // Condition arms contain arbitrary compile-time Exprs — skip for scope checking
                    WhenArm::Cond { .. } => {}
                    // `is <TypeExpr> then <body>` — check pattern for unknown symbols
                    WhenArm::Is { pattern, .. } => {
                        if let Some((name, span)) = pattern.value.nominal_with_span()
                            && name.as_str().is_capitalized_type_name()
                            && !type_scope.contains(name)
                        {
                            push_flaw(
                                span,
                                // type-unknown-symbol
                                Diagnostic::new(
                                    SymbolCode::Unknown,
                                    format!("unknown symbol `{}`", name.as_str()),
                                )
                                .with_arg("name", name.as_str().to_string()),
                            );
                        }
                    }
                    WhenArm::Else(..) => {}
                }
            }
        }
        _ => {}
    }
}

fn check_qualified_has_properties(
    declare: &ast::Declare,
    push_flaw: &mut dyn FnMut(SpanId, Diagnostic),
) {
    let DeclareValue::Has(members) = &declare.value else {
        return;
    };

    for member in members {
        let qualifier = match member {
            ast::HasMember::Property(property) => property.qualifier.as_ref(),
            ast::HasMember::Function(_) => continue,
        };
        let Some(qualifier) = qualifier else {
            continue;
        };
        if qualifier.name == declare.name {
            continue;
        };
        if !declare
            .provided_traits
            .iter()
            .any(|provided| provided.trait_name == qualifier.name)
        {
            push_flaw(
                qualifier.span,
                // type-uninherited-trait-qualifier
                Diagnostic::new(
                    TraitCode::UninheritedQualifier,
                    format!(
                        "`{}` does not inherit trait `{}`",
                        declare.name.as_str(),
                        qualifier.name.as_str()
                    ),
                )
                .with_arg("type_name", declare.name.as_str().to_string())
                .with_arg("trait_name", qualifier.name.as_str().to_string()),
            );
            continue;
        }
    }
}

fn check_declare_value_type_scope(
    declare_value: &DeclareValue,
    type_scope: &HashSet<Intern<String>>,
    push_flaw: &mut dyn FnMut(SpanId, Diagnostic),
) {
    check_union_variant_shape_type_scope(declare_value, type_scope, push_flaw);
    check_has_associated_method_self(declare_value, push_flaw);
}

fn check_has_associated_method_self(
    declare_value: &DeclareValue,
    push_flaw: &mut dyn FnMut(SpanId, Diagnostic),
) {
    let DeclareValue::Has(members) = declare_value else {
        return;
    };

    for member in members {
        let ast::HasMember::Function(function) = member else {
            continue;
        };
        if function.kind != ast::HasFunctionKind::Associated {
            continue;
        }
        let Some(body) = &function.body else {
            continue;
        };
        let value = match body {
            ast::HasMemberBody::Overrideable(value) | ast::HasMemberBody::Final(value) => value,
        };
        let mut spans = Vec::new();
        collect_bind_value_self_ref_spans(value, &mut spans);
        for span in spans {
            push_flaw(
                span,
                // no-self-in-associated-method
                Diagnostic::new(
                    CallCode::AssociatedMethodMissingSelf,
                    "associated method has no `self` parameter",
                )
                .with_help("add a `self`, `self ref`, or `self mut` parameter, or call through an explicit value"),
            );
        }
    }
}

pub(crate) fn collect_bind_value_self_ref_spans(value: &BindValue, spans: &mut Vec<SpanId>) {
    match value {
        BindValue::Expr(expr) => collect_self_ref_spans(expr, spans),
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                collect_self_ref_spans(expr, spans);
            }
            for expr in &ret.values {
                collect_self_ref_spans(expr, spans);
            }
        }
        BindValue::Unassigned => {}
    }
}

pub(crate) fn collect_self_ref_spans(expr: &Typed<Expr>, spans: &mut Vec<SpanId>) {
    fn collect(expr: &Expr, span_id: SpanId, spans: &mut Vec<SpanId>) {
        if matches!(expr, Expr::SelfRef) {
            spans.push(span_id);
        }
        let _ = ast::folder::walk_expr_children(expr, &mut |span_id, child| {
            collect(child, span_id, spans);
            std::ops::ControlFlow::Continue(())
        });
    }
    collect(&expr.value, expr.span_id, spans);
}

fn check_params_for_in_range_on_bounded_tag(
    params: Option<&Parameters>,
    tag_types: &HashMap<TagId, Ty>,
    flaws: &mut Vec<(SpanId, Diagnostic)>,
) {
    let Some(params) = params else {
        return;
    };
    for (_, kind) in params {
        let ParameterKind::Tagged(sp) = &kind.kind else {
            continue;
        };
        let Expr::Range(range) = &sp.value else {
            continue;
        };
        let Expr::AnonymousTag(tag) = &range.start.value else {
            continue;
        };
        let Expr::AnonymousTag(end) = &range.end.value else {
            continue;
        };
        if tag != end {
            continue;
        }
        if tag_types
            .get(&TagId(*tag))
            .is_some_and(|ty| ty.is_bounded_int())
        {
            flaws.push((
                range.start.span_id,
                // type-in-range-on-bounded-int-tag
                Diagnostic::new(
                    IntegerCode::BoundedTagRangeConstraint,
                    format!(
                        "`in {}(...)` on a bounded int tag `{}`",
                        tag.as_str(),
                        tag.as_str()
                    ),
                )
                .with_arg("tag", tag.as_str().to_string()),
            ));
        }
    }
}

fn tag_params_map(tags: &ast::TagMap) -> HashMap<Intern<String>, Parameters> {
    let mut map = HashMap::new();
    for (name, decl) in tags {
        if let Some(params) = &decl.params {
            map.insert(*name, params.clone());
        }
    }
    map
}

fn references_nominal_type(ty: &Ty, target: Intern<String>) -> bool {
    matches!(ty, Ty::Named { name, .. } | Ty::Record { name, .. } | Ty::Union { name, .. } | Ty::Opaque(name) if *name == target)
        || ty.any_child(|child| {
            child
                .ty()
                .is_some_and(|ty| references_nominal_type(ty, target))
        })
}

fn resolve_tag_pass(
    tag_name: &Intern<String>,
    declare: &ast::declare::Declare,
    all_tags: TagDeclarations<'_>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    cross_file_tag_types: &HashMap<TagId, Ty>,
    resolved: &HashMap<Intern<String>, Ty>,
    file_id: FileId,
) -> Ty {
    let acyclic_resolved = resolved
        .iter()
        .filter(|(name, ty)| **name != *tag_name && !references_nominal_type(ty, *tag_name))
        .map(|(name, ty)| (*name, ty.clone()))
        .collect();
    let instance = ast::ty::NamedTypeInstance::new(ast::ty::TypeId {
        file: file_id.0,
        name: *tag_name,
    })
    .with_arguments(declaration_instance_arguments(declare));
    let resolved = resolve_declare_value(
        tag_name,
        &declare.value,
        all_tags,
        tag_params,
        cross_file_tag_types,
        &acyclic_resolved,
        file_id,
    );
    preserve_self_reference_with_instance(&resolved, *tag_name, &instance)
}

pub(super) fn nominal_type(declare: &ast::Declare, file_id: FileId) -> Option<Ty> {
    if matches!(&declare.value, DeclareValue::Alias(alias)
        if !matches!(&alias.value, Expr::TakePtr(_) | Expr::IntegerDomain(_)))
    {
        return None;
    }
    Some(Ty::Named {
        instance: ast::ty::NamedTypeInstance::new(ast::ty::TypeId {
            file: file_id.0,
            name: declare.name,
        })
        .with_arguments(declaration_instance_arguments(declare)),
        name: declare.name,
    })
}

fn nominal_types_for_definitions(
    definitions: &HashMap<Intern<String>, Ty>,
    tags: &ast::TagMap,
    file_id: FileId,
) -> HashMap<Intern<String>, Ty> {
    definitions
        .iter()
        .map(|(name, definition)| {
            let Some(declare) = tags.get(name) else {
                return (*name, definition.clone());
            };
            (
                *name,
                nominal_type(declare, file_id).unwrap_or_else(|| definition.clone()),
            )
        })
        .collect()
}

pub(crate) fn resolve_tags_fixpoint(
    tags: &ast::TagMap,
    all_tags: TagDeclarations<'_>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    cross_file_tag_types: &HashMap<TagId, Ty>,
    file_id: FileId,
    max_passes: usize,
    completed_aliases: &HashMap<TagId, Ty>,
) -> HashMap<Intern<String>, Ty> {
    let completed: HashMap<_, _> = tags
        .iter()
        .filter_map(|(name, declare)| {
            let ty = completed_aliases.get(&TagId(*name))?;
            let instance = ast::ty::NamedTypeInstance::new(ast::ty::TypeId {
                file: file_id.0,
                name: *name,
            })
            .with_arguments(declaration_instance_arguments(declare));
            Some((
                *name,
                preserve_self_reference_with_instance(ty, *name, &instance),
            ))
        })
        .collect();
    if completed.len() == tags.len() {
        return completed;
    }
    let mut resolved: HashMap<Intern<String>, Ty> = completed.clone();
    for _pass in 0..max_passes {
        let available = nominal_types_for_definitions(&resolved, tags, file_id);
        let next: HashMap<_, _> = tags
            .iter()
            .map(|(name, declare)| {
                (
                    *name,
                    completed.get(name).cloned().unwrap_or_else(|| {
                        resolve_tag_pass(
                            name,
                            declare,
                            all_tags,
                            tag_params,
                            cross_file_tag_types,
                            &available,
                            file_id,
                        )
                    }),
                )
            })
            .collect();
        if next == resolved {
            break;
        }
        resolved = next;
    }
    resolved
}

pub(crate) fn rebuild_own_variant_map(
    variant_map: &mut VariantMap,
    tag_types: &HashMap<TagId, Ty>,
    own_unions: &HashSet<Intern<String>>,
    type_registry: &crate::TypeRegistry,
) {
    variant_map.retain(|_, _| false);
    for (tag_id, ty) in tag_types {
        if own_unions.contains(&tag_id.0) {
            push_union_variants_to_map(variant_map, tag_id.0, ty, type_registry);
        }
    }
}

fn push_union_variants_to_map(
    variant_map: &mut VariantMap,
    union_name: Intern<String>,
    resolved_ty: &Ty,
    type_registry: &crate::TypeRegistry,
) {
    match type_registry.resolved_definition_for_type(resolved_ty) {
        Ty::Union {
            variants,
            literal_values: None,
            ..
        } => {
            for (disc, v) in variants.iter().enumerate() {
                let fields: Vec<(Intern<String>, Ty)> =
                    v.fields.iter().map(|(n, t)| (*n, *t.clone())).collect();
                variant_map
                    .entry(v.name)
                    .or_default()
                    .push((union_name, disc, fields));
            }
        }
        Ty::Union {
            literal_values: Some(values),
            ..
        } => {
            for (disc, cv) in values.iter().enumerate() {
                let name = cv.display_name();
                variant_map
                    .entry(name)
                    .or_default()
                    .push((union_name, disc, vec![]));
            }
        }
        _ => {}
    }
}

fn literal_const_from_type_expr(expr: &Pattern) -> Option<ConstValue> {
    match expr {
        Pattern::Literal(Literal::String(s), _) => Some(ConstValue::Text(s.clone())),
        Pattern::Literal(Literal::Int(n), _) => Some(ConstValue::Int(*n)),
        Pattern::Literal(Literal::Float(HashFloat(f)), _) => Some(ConstValue::Float(HashFloat(*f))),
        _ => None,
    }
}

fn resolve_declare_value(
    declaring_name: &Intern<String>,
    value: &DeclareValue,
    all_tags: TagDeclarations<'_>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    cross_file_tag_types: &HashMap<TagId, Ty>,
    resolved: &HashMap<Intern<String>, Ty>,
    _file_id: FileId,
) -> Ty {
    let tag_types = resolved_types(resolved, cross_file_tag_types);
    let env = TypeEnv::new(&tag_types)
        .with_tag_params(tag_params)
        .with_tag_declaration_lookup(all_tags);

    match value {
        DeclareValue::Alias(sp) => env.resolve_expr(&sp.value),
        DeclareValue::Union { variants } => {
            let literal_values: Option<Vec<ConstValue>> = variants
                .iter()
                .map(|v| literal_const_from_type_expr(&v.shape().value))
                .collect();
            if let Some(values) = literal_values {
                return Ty::union_of_literals(*declaring_name, values);
            }

            let resolved_variants: Vec<UnionVariant> = variants
                .iter()
                .map(|v| {
                    let shape = v.shape();
                    if let Pattern::Generic { name, params, .. } = &shape.value
                        && env.tag_types.contains_key(name)
                    {
                        let ty = env.resolve_generic(name, params);
                        return UnionVariant {
                            kind: ast::ty::UnionVariantKind::Type,
                            discriminant: v.discriminant().map(|value| value.value),
                            name: *name,
                            fields: vec![(Intern::from_ref("value"), Box::new(ty))],
                            result_ty: v.result_ty().map(|rt| env.resolve_expr(&rt.value)),
                        };
                    }
                    let (name, fields) = match &shape.value {
                        Pattern::Generic { name, params, .. } => {
                            let fields: Vec<(Intern<String>, Box<Ty>)> = params
                                .iter()
                                .map(|(name, kind)| {
                                    let ty = match kind {
                                        ParameterKind::Tagged(sp) => env.resolve_expr(&sp.value),
                                        ParameterKind::ValueParam { ty }
                                        | ParameterKind::Inferred { ty } => {
                                            env.resolve_expr(&ty.value)
                                        }
                                        ParameterKind::Proposition(predicate) => {
                                            Ty::Proposition(predicate.clone())
                                        }
                                        ParameterKind::Generic => Ty::Opaque(*name),
                                        ParameterKind::Default(expr)
                                            if expr_is_type_surface(&expr.value) =>
                                        {
                                            env.resolve_expr(&expr.value)
                                        }
                                        ParameterKind::Default(expr) => {
                                            let infer_env = TyInferEnv {
                                                tag_types: env.tag_types,
                                                fn_return_types: &HashMap::new(),
                                                locals: &HashMap::new(),
                                                tag_params: env.tag_params,
                                            };
                                            expr.infer_ty(&infer_env)
                                        }
                                    };
                                    (*name, Box::new(ty))
                                })
                                .collect();
                            (*name, fields)
                        }
                        Pattern::Nominal(name, _) => {
                            let fields = env
                                .tag_types
                                .get(name)
                                .map(|ty| vec![(Intern::from_ref("value"), Box::new(ty.clone()))])
                                .unwrap_or_default();
                            (*name, fields)
                        }
                        _ => (
                            Intern::new(String::new()),
                            Vec::<(Intern<String>, Box<Ty>)>::new(),
                        ),
                    };
                    let result_ty = v.result_ty().map(|rt| env.resolve_expr(&rt.value));
                    UnionVariant {
                        discriminant: v.discriminant().map(|value| value.value),
                        kind: if matches!(shape.value, Pattern::Nominal(_, _)) && !fields.is_empty()
                        {
                            ast::ty::UnionVariantKind::Type
                        } else {
                            ast::ty::UnionVariantKind::Constructor
                        },
                        name,
                        fields,
                        result_ty,
                    }
                })
                .collect();
            Ty::Union {
                name: *declaring_name,
                variants: resolved_variants,
                literal_values: None,
                resolved_params: None,
            }
        }

        DeclareValue::InRange(start, end) => Ty::bounded_int(*start, *end),
        DeclareValue::Refinement(predicate) => Ty::AnonymousInteger {
            validity: IntegerValidity::new(IntegerDomain::from_refinement(predicate)),
        },
        DeclareValue::Range(start, end) => {
            Ty::range_value_record(Intern::new("range_value".to_string()), *start, *end)
        }
        DeclareValue::Set() => Ty::Unit,
        DeclareValue::Has(members) => {
            let resolved_fields: Vec<(Intern<String>, Box<Ty>)> = members
                .iter()
                .filter_map(|m| match m {
                    HasMember::Property(p)
                        if p.qualifier
                            .as_ref()
                            .is_none_or(|qualifier| qualifier.name == *declaring_name) =>
                    {
                        let field_ty =
                            p.ty.as_ref()
                                .map(|ty| env.resolve_expr(&ty.value))
                                .unwrap_or(Ty::Unit);
                        let field_ty = if p.owned_storage {
                            Ty::OwnedStorage {
                                inner: Box::new(field_ty),
                            }
                        } else {
                            field_ty
                        };
                        Some((p.name, Box::new(field_ty)))
                    }
                    HasMember::Property(_) | HasMember::Function(_) => None,
                })
                .collect();
            Ty::Record {
                name: *declaring_name,
                fields: resolved_fields,
                resolved_params: None,
            }
        }
        DeclareValue::When(when) => {
            if let Some(subject) = &when.subject {
                let infer_env = TyInferEnv {
                    tag_types: env.tag_types,
                    fn_return_types: &HashMap::new(),
                    locals: &HashMap::new(),
                    tag_params: env.tag_params,
                };
                let subject_ty = if expr_is_type_surface(&subject.value) {
                    env.resolve(&subject.value)
                } else {
                    subject.infer_ty(&infer_env)
                };
                // Try to simplify via compile-time constant folding
                for arm in &when.arms {
                    if let WhenArm::Is { body, .. } = arm {
                        let body_ty = if expr_is_type_surface(&body.value) {
                            env.resolve(&body.value)
                        } else {
                            body.infer_ty(&infer_env)
                        };
                        if body_ty != Ty::Unit {
                            return body_ty;
                        }
                    }
                }
                subject_ty
            } else {
                Ty::Unit
            }
        }
    }
}

fn signature_tag_types(
    own: &HashMap<TagId, Ty>,
    cross_file: &HashMap<TagId, Ty>,
) -> HashMap<Intern<String>, Ty> {
    let own = own.iter().map(|(id, ty)| (id.0, ty.clone())).collect();
    resolved_types(&own, cross_file)
}

fn resolved_types<'a>(
    resolved: &'a HashMap<Intern<String>, Ty>,
    cross_file: &'a HashMap<TagId, Ty>,
) -> HashMap<Intern<String>, Ty> {
    let mut map: HashMap<Intern<String>, Ty> = resolved.clone();
    for (tid, ty) in cross_file {
        map.entry(tid.0).or_insert_with(|| ty.clone());
    }
    map
}

fn resolve_type_from_typed_expr(expr: &Typed<Expr>, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    match &expr.value {
        Expr::AnonymousTag(name) => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        Expr::Lit(Literal::Int(value)) => Ty::AnonymousInteger {
            validity: IntegerValidity::new(
                IntegerDomain::bounded(*value, *value).expect("singleton integer domain"),
            ),
        },
        Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elements)) => Ty::sequence(
            elements
                .iter()
                .map(|element| ast::ty::SequenceTypeElement {
                    name: element.name,
                    ty: Box::new(resolve_type_from_typed_expr(&element.expr, tag_types)),
                })
                .collect(),
        ),
        Expr::Lit(_) | Expr::TagCall(_) => expr.infer_ty(&TyInferEnv {
            tag_types,
            fn_return_types: &HashMap::new(),
            locals: &HashMap::new(),
            tag_params: None,
        }),
        _ => Ty::Unit,
    }
}

fn resolve_target_groups(
    bind: &Bind,
    param_types: &[(Intern<String>, Ty)],
    param_conventions: &[ParamConvention],
    type_registry: &crate::type_registry::TypeRegistry,
    flaws: &mut Vec<(SpanId, Diagnostic)>,
) -> (Vec<Option<GroupId>>, Vec<TypedGroup>, Vec<Option<GroupId>>) {
    let mut named: HashMap<ast::GroupPath, GroupId> = HashMap::new();
    let mut groups: Vec<TypedGroup> = Vec::new();
    let param_groups = param_types
        .iter()
        .zip(param_conventions)
        .map(|((param_name, param_type), convention)| {
            if !matches!(
                convention,
                ParamConvention::Observe | ParamConvention::Mutate
            ) {
                return None;
            }

            let referent_type = reference_semantics_for_type(param_type, Some(type_registry))
                .map(|reference| reference.pointee)
                .unwrap_or_else(|| param_type.clone());
            let group_name = bind.param_groups.get(param_name).cloned();
            if let Some(group_name) = group_name.as_ref()
                && let Some(group_id) = named.get(group_name).copied()
            {
                if !same_group_referent(
                    &groups[group_id.0 as usize].referent_type,
                    &referent_type,
                    type_registry,
                ) {
                    flaws.push((
                        bind.name_span,
                        // type-incompatible-target-group
                        Diagnostic::new(
                            TargetGroupCode::Incompatible,
                            format!(
                                "target group `{}` is used with incompatible referent types",
                                group_name
                            ),
                        )
                        .with_arg("group", group_name.to_string()),
                    ));
                }
                return Some(group_id);
            }

            let group_id = GroupId(groups.len() as u32);
            groups.push(TypedGroup {
                projections: group_name
                    .as_ref()
                    .filter(|path| path.segments.is_empty())
                    .map(|_| Vec::new()),
                path: group_name.clone(),
                referent_type: referent_type.clone(),
            });
            if let Some(group_name) = group_name {
                named.insert(group_name, group_id);
            }
            Some(group_id)
        })
        .collect();

    let mut missing_roots = HashSet::new();
    for group in &groups {
        let Some(path) = group.path.as_ref().filter(|path| !path.segments.is_empty()) else {
            continue;
        };
        let root = ast::GroupPath::root(path.root);
        if !named.contains_key(&root) && missing_roots.insert(root.clone()) {
            flaws.push((
                bind.name_span,
                // type-unknown-target-group
                Diagnostic::new(
                    TargetGroupCode::Unknown,
                    format!("derived target group `{path}` has no root group `{root}`"),
                )
                .with_arg("group", path.to_string())
                .with_arg("root", root.to_string()),
            ));
        }
    }

    let root_types: HashMap<_, _> = groups
        .iter()
        .filter_map(|group| {
            let path = group.path.as_ref()?;
            path.segments
                .is_empty()
                .then_some((path.root, group.referent_type.clone()))
        })
        .collect();
    for group in &mut groups {
        let Some(path) = group.path.as_ref().filter(|path| !path.segments.is_empty()) else {
            continue;
        };
        let Some(root_type) = root_types.get(&path.root) else {
            continue;
        };
        match resolve_group_path(root_type, path, type_registry) {
            Ok((projections, resolved_type)) => {
                group.projections = Some(projections);
                if !same_group_referent(&resolved_type, &group.referent_type, type_registry) {
                    flaws.push((
                        bind.name_span,
                        // type-incompatible-target-group
                        Diagnostic::new(
                            TargetGroupCode::Incompatible,
                            format!(
                                "target group `{path}` resolves to `{}`, but is used with `{}`",
                                resolved_type.format_for_hover(),
                                group.referent_type.format_for_hover()
                            ),
                        )
                        .with_arg("group", path.to_string())
                        .with_arg("resolved_type", resolved_type.format_for_hover())
                        .with_arg("referent_type", group.referent_type.format_for_hover()),
                    ));
                }
            }
            Err(error) => flaws.push((bind.name_span, error.diagnostic(path))),
        }
    }

    let return_group = match bind.return_tag.as_deref() {
        Some(spanned) => match &spanned.value {
            Expr::Ref {
                group: Some(name), ..
            } => match named.get(name).copied() {
                Some(group_id) => Some(group_id),
                None => {
                    flaws.push((
                        spanned.span_id,
                        // type-unknown-target-group
                        Diagnostic::new(
                            TargetGroupCode::Unknown,
                            format!("unknown target group `{name}`"),
                        )
                        .with_arg("group", name.to_string()),
                    ));
                    None
                }
            },
            Expr::Ref { group: None, .. } => match groups.len() {
                1 => Some(GroupId(0)),
                0 => None,
                count => {
                    flaws.push((
                        spanned.span_id,
                        // type-ambiguous-reference-return-target-group
                        Diagnostic::new(
                            ReferenceCode::AmbiguousReturnTargetGroup,
                            "reference return has multiple eligible reference input groups",
                        )
                        .with_arg("group_count", count.to_string()),
                    ));
                    None
                }
            },
            _ => None,
        },
        None => None,
    };

    let result_groups = bind
        .named_results
        .iter()
        .map(|result| match &result.ty.value {
            Expr::Ref {
                group: Some(name), ..
            } => named.get(name).copied(),
            Expr::Ref { group: None, .. } if groups.len() == 1 => Some(GroupId(0)),
            _ => None,
        })
        .collect();

    let result_groups = if bind.named_results.is_empty() {
        vec![return_group]
    } else {
        result_groups
    };
    (param_groups, groups, result_groups)
}

#[derive(Debug, Clone, PartialEq)]
enum GroupPathError {
    UnknownField(Intern<String>),
    UnknownVariant(Intern<String>),
    InvalidProjection {
        segment: Intern<String>,
        ty: Box<Ty>,
    },
}

impl GroupPathError {
    fn diagnostic(&self, path: &ast::GroupPath) -> Diagnostic {
        match self {
            // type-unknown-target-group-field
            Self::UnknownField(field) => Diagnostic::new(
                TargetGroupCode::UnknownField,
                format!("target group `{path}` refers to unknown field `{field}`"),
            )
            .with_arg("group", path.to_string())
            .with_arg("field", field.to_string()),
            // type-unknown-target-group-variant
            Self::UnknownVariant(variant) => Diagnostic::new(
                TargetGroupCode::UnknownVariant,
                format!("target group `{path}` refers to unknown variant `{variant}`"),
            )
            .with_arg("group", path.to_string())
            .with_arg("variant", variant.to_string()),
            // type-invalid-target-group-projection
            Self::InvalidProjection { segment, ty } => Diagnostic::new(
                TargetGroupCode::InvalidProjection,
                format!(
                    "target group segment `{segment}` cannot project from `{}`",
                    ty.format_for_hover()
                ),
            )
            .with_arg("group", path.to_string())
            .with_arg("segment", segment.to_string())
            .with_arg("type", ty.format_for_hover()),
        }
    }
}

enum GroupPathCursor {
    Type(Ty),
    Variant(Vec<(Intern<String>, Box<Ty>)>),
}

impl GroupPathCursor {
    fn into_type(self) -> Ty {
        match self {
            Self::Type(ty) => ty,
            Self::Variant(fields) => Ty::Record {
                name: Intern::from_ref("Record"),
                fields,
                resolved_params: None,
            },
        }
    }

    fn unwrapped(self, type_registry: &crate::type_registry::TypeRegistry) -> Self {
        let Self::Type(mut ty) = self else {
            return self;
        };
        let mut seen = HashSet::new();
        loop {
            if let Ty::Named { instance, .. } = &ty
                && !seen.insert(instance.declaration)
            {
                return Self::Type(ty);
            }
            if let Some(reference) = reference_semantics_for_type(&ty, Some(type_registry)) {
                ty = reference.pointee;
            } else if matches!(ty, Ty::Named { .. }) {
                ty = type_registry.resolved_definition_for_type(&ty);
            } else {
                return Self::Type(ty);
            }
        }
    }
}

fn same_group_referent(left: &Ty, right: &Ty, type_registry: &crate::TypeRegistry) -> bool {
    match (left.named_instance(), right.named_instance()) {
        (Some(left), Some(right)) => left == right,
        (Some(_), None) => type_registry.resolved_definition_for_type(left) == *right,
        (None, Some(_)) => *left == type_registry.resolved_definition_for_type(right),
        (None, None) => left == right,
    }
}

fn resolve_group_path(
    root_type: &Ty,
    path: &ast::GroupPath,
    type_registry: &crate::type_registry::TypeRegistry,
) -> Result<(Vec<GroupProjection>, Ty), GroupPathError> {
    let mut cursor = GroupPathCursor::Type(root_type.clone());
    let mut projections = Vec::with_capacity(path.segments.len());
    for segment in &path.segments {
        cursor = cursor.unwrapped(type_registry);
        let invalid_projection = |cursor: GroupPathCursor| GroupPathError::InvalidProjection {
            segment: *segment,
            ty: Box::new(cursor.into_type()),
        };
        match segment.as_str() {
            "items" => match cursor {
                GroupPathCursor::Type(Ty::Array { elem, .. }) => {
                    projections.push(GroupProjection::Items);
                    cursor = GroupPathCursor::Type(*elem);
                }
                cursor => return Err(invalid_projection(cursor)),
            },
            "pointee" => match cursor {
                GroupPathCursor::Type(ty) if ty.pointee_ty().is_some() => {
                    projections.push(GroupProjection::Pointee);
                    cursor = GroupPathCursor::Type(ty.pointee_ty().cloned().unwrap());
                }
                cursor => return Err(invalid_projection(cursor)),
            },
            _ if segment
                .as_str()
                .chars()
                .next()
                .is_some_and(char::is_uppercase) =>
            {
                match cursor {
                    GroupPathCursor::Type(Ty::Union { variants, .. }) => {
                        let Some(variant) = variants
                            .into_iter()
                            .find(|variant| variant.name == *segment)
                        else {
                            return Err(GroupPathError::UnknownVariant(*segment));
                        };
                        projections.push(GroupProjection::Variant { name: *segment });
                        cursor = GroupPathCursor::Variant(variant.fields);
                    }
                    cursor => return Err(invalid_projection(cursor)),
                }
            }
            _ => match cursor {
                GroupPathCursor::Type(Ty::Record { fields, .. })
                | GroupPathCursor::Variant(fields) => {
                    let Some((index, (_, field_type))) = fields
                        .into_iter()
                        .enumerate()
                        .find(|(_, (name, _))| name == segment)
                    else {
                        return Err(GroupPathError::UnknownField(*segment));
                    };
                    projections.push(GroupProjection::Field {
                        name: *segment,
                        index,
                    });
                    cursor = GroupPathCursor::Type(*field_type);
                }
                cursor => return Err(invalid_projection(cursor)),
            },
        }
    }
    Ok((projections, cursor.unwrapped(type_registry).into_type()))
}

#[allow(clippy::too_many_arguments)]
fn resolve_return_type(
    bind: &Bind,
    registry: &crate::TypeRegistry,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: TagDeclarations<'_>,
    dependent_binder: BinderId,
    typevar_subst: &HashMap<Intern<String>, Ty>,
    receiver_type: Option<&Ty>,
    receiver_type_surface: Option<&ast::Expr>,
    result_family_owner: &ast::ResultFamilyOwner,
) -> Ty {
    let normalized;
    let bind = if let BindValue::Body { exprs, ret } = &bind.value
        && exprs.is_empty()
        && let [value] = ret.values.as_slice()
    {
        normalized = {
            let mut bind = bind.clone();
            bind.value = BindValue::Expr(Box::new(value.clone()));
            bind
        };
        &normalized
    } else {
        bind
    };
    if let Some(ty) = TypeEnv::new(tag_types)
        .with_registry(registry)
        .with_subst(typevar_subst)
        .with_tag_params(tag_params)
        .with_tag_declaration_lookup(tag_decls)
        .with_dependent_binder(dependent_binder)
        .resolve_result_alternatives(&bind.anonymous_result_alternatives)
    {
        return ty;
    }
    if !bind.anonymous_result_alternatives.is_empty() {
        let mut alternatives = Vec::new();
        for alternative in &bind.anonymous_result_alternatives {
            if alternatives
                .iter()
                .all(|existing: &ast::ResultAlternative| existing.label != alternative.value.label)
            {
                alternatives.push(alternative.value.clone());
            }
        }
        return Ty::ResultFamily {
            owner: result_family_owner.clone(),
            alternatives,
        };
    }
    let inferred_alternatives = if bind.return_tag.is_none()
        && bind.return_type_name.is_none()
        && bind.type_annotation.is_none()
    {
        inferred_result_alternatives(&bind.value, tag_decls)
    } else {
        Vec::new()
    };
    if !inferred_alternatives.is_empty() {
        return Ty::ResultFamily {
            owner: result_family_owner.clone(),
            alternatives: inferred_alternatives
                .into_iter()
                .map(|label| ast::ResultAlternative {
                    label,
                    proposition: super::postconditions::alternative(bind, label),
                })
                .collect(),
        };
    }
    if !bind.named_results.is_empty() {
        let env = TypeEnv::new(tag_types)
            .with_registry(registry)
            .with_subst(typevar_subst)
            .with_tag_params(tag_params)
            .with_tag_declaration_lookup(tag_decls)
            .with_dependent_binder(dependent_binder);
        let elements = bind
            .named_results
            .iter()
            .map(|result| ast::ty::SequenceTypeElement {
                name: result.name,
                ty: Box::new(env.resolve_expr(&result.ty.value)),
            })
            .collect::<Vec<_>>();
        return Ty::results(elements);
    }
    if bind.return_tag.is_none()
        && let Some(ast::PredicateExpr::Proposition(predicate)) = &bind.return_refinement
    {
        return Ty::Proposition(predicate.clone());
    }
    if let Some(predicate) = &bind.return_refinement {
        return Ty::AnonymousInteger {
            validity: ast::integer::IntegerValidity::new(
                ast::integer::IntegerDomain::from_refinement(predicate),
            ),
        };
    }
    if let Some(sp) = &bind.return_tag
        && expr_is_type_surface(&sp.value)
    {
        if let (Some(receiver_type), Some(receiver_surface)) =
            (receiver_type, receiver_type_surface)
            && *receiver_surface == sp.value
        {
            return receiver_type.clone();
        }
        if matches!(&sp.value, ast::Expr::AnonymousTag(name) if name.as_str() == "Self")
            && let Some(receiver_type) = receiver_type
        {
            return receiver_type.clone();
        }
        if let ast::Expr::TagCall(tag_call) = &sp.value
            && tag_call.name.as_str() == "Self"
            && let Some(receiver_type) = receiver_type
        {
            return receiver_type.clone();
        }
        let ty = TypeEnv::new(tag_types)
            .with_registry(registry)
            .with_subst(typevar_subst)
            .with_tag_params(tag_params)
            .with_tag_declaration_lookup(tag_decls)
            .with_dependent_binder(dependent_binder)
            .resolve(&sp.value);
        return nominalize_explicit_ty_surface(&sp.value, ty);
    }
    if let Some((name, args)) = &bind.type_annotation {
        let annotated = ast::Expr::TagCall(ast::expr::TagCall {
            name: *name,
            qual_path: bind.type_annotation_qual.clone(),
            args: args.clone(),
        });
        let ty = TypeEnv::new(tag_types)
            .with_registry(registry)
            .with_subst(typevar_subst)
            .with_tag_params(tag_params)
            .with_tag_declaration_lookup(tag_decls)
            .with_dependent_binder(dependent_binder)
            .resolve(&annotated);
        return nominalize_explicit_ty_surface(&annotated, ty);
    }
    if let Some(name) = &bind.return_type_name {
        let ty = tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name));
        return nominalize_explicit_ty_surface(&ast::Expr::AnonymousTag(*name), ty);
    }
    // Try to infer from body
    match &bind.value {
        BindValue::Expr(expr) => {
            let forwarded_parameter = match &expr.value {
                ast::Expr::FnCall(call)
                    if call.args.is_none() && call.path.value.segments.is_empty() =>
                {
                    Some(call.path.value.root)
                }
                ast::Expr::AnonymousTag(name) => Some(*name),
                _ => None,
            };
            if let Some(parameter) = forwarded_parameter
                && let Some(kind) = bind
                    .params
                    .as_ref()
                    .and_then(|params| params.get(&parameter))
            {
                return match &kind.kind {
                    ParameterKind::Generic
                        if super::parameters::is_inferred_value(bind, parameter) =>
                    {
                        super::parameters::inferred_value_type(bind, parameter)
                    }
                    ParameterKind::Proposition(predicate) => Ty::Proposition(predicate.clone()),
                    ParameterKind::Generic => Ty::Opaque(parameter),
                    ParameterKind::Tagged(surface)
                    | ParameterKind::ValueParam { ty: surface }
                    | ParameterKind::Inferred { ty: surface } => TypeEnv::new(tag_types)
                        .with_registry(registry)
                        .with_subst(typevar_subst)
                        .with_tag_params(tag_params)
                        .with_tag_declaration_lookup(tag_decls)
                        .with_dependent_binder(dependent_binder)
                        .resolve_expr(&surface.value),
                    ParameterKind::Default(default) => {
                        let env = TyInferEnv {
                            tag_types,
                            fn_return_types: &HashMap::new(),
                            locals: &HashMap::new(),
                            tag_params: Some(tag_params),
                        };
                        default.infer_ty(&env)
                    }
                };
            }
            if let ast::Expr::TagCall(tag_call) = &expr.value
                && tag_call.name.as_str() == "Self"
                && let Some(receiver_type) = receiver_type
            {
                return receiver_type.clone();
            }
            resolve_type_from_typed_expr(expr, tag_types)
        }
        BindValue::Body { ret, .. } => {
            if !ret.values.is_empty() {
                let fields = ret
                    .values
                    .iter()
                    .map(|expr| ast::ty::SequenceTypeElement {
                        name: None,
                        ty: Box::new(resolve_type_from_typed_expr(expr, tag_types)),
                    })
                    .collect();
                Ty::results(fields)
            } else {
                ret.values
                    .first()
                    .map(|e| resolve_type_from_typed_expr(e, tag_types))
                    .unwrap_or(Ty::Unit)
            }
        }
        _ => Ty::Unit,
    }
}

fn inferred_result_alternatives(
    value: &BindValue,
    tag_decls: TagDeclarations<'_>,
) -> Vec<Intern<String>> {
    let mut alternatives = Vec::new();
    match value {
        BindValue::Expr(expr) => {
            collect_result_exit_alternatives(&expr.value, tag_decls, &mut alternatives)
        }
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                if matches!(expr.value, Expr::If(_) | Expr::Loop(_) | Expr::When(_)) {
                    collect_result_exit_alternatives(&expr.value, tag_decls, &mut alternatives);
                }
            }
            for ret in &ret.values {
                collect_result_exit_alternatives(&ret.value, tag_decls, &mut alternatives);
            }
        }
        BindValue::Unassigned => {}
    }
    alternatives
}

fn validate_result_proof_surfaces(
    bind: &Bind,
    compile_time_ast: &ast::FileAst,
    flaws: &mut Vec<Diagnostic>,
    span_table: &ast::SpanTable,
) {
    let allowed_names = bind
        .params
        .iter()
        .flat_map(|params| params.keys().copied())
        .chain(bind.is_method().then_some(Intern::from_ref("self")))
        .chain(
            compile_time_ast
                .defs
                .iter()
                .filter_map(|(name, bind)| bind.is_constant().then_some(name.0)),
        )
        .collect::<HashSet<_>>();
    let mut propositions = HashMap::new();
    for alternative in &bind.anonymous_result_alternatives {
        let Some(proposition) = &alternative.value.proposition else {
            continue;
        };
        if let Some(previous) = propositions.insert(alternative.value.label, proposition)
            && previous != proposition
        {
            flaws.push(
                // type-result-proof-conflict
                Diagnostic::new(
                    ResultCode::ProofConflict,
                    format!(
                        "alternative `{}` has conflicting `thus` contracts",
                        alternative.value.label
                    ),
                )
                .with_arg("alternative", alternative.value.label.as_str().to_string())
                .at_span_id(alternative.span_id, span_table),
            );
        }
        let mut names = HashSet::new();
        let _ = ProofNameCollector(&mut names).visit_proposition(proposition);
        for name in names.difference(&allowed_names) {
            flaws.push(
                // type-proof-contract-unknown-name
                Diagnostic::new(
                    ProofCode::ProofContractUnknownName,
                    format!("proof contract refers to unknown incoming value `{name}`"),
                )
                .with_arg("name", name.as_str().to_string())
                .at_span_id(alternative.span_id, span_table),
            );
        }
    }
    verify_result_constructions(bind, flaws, span_table);
}

fn verify_result_constructions(
    bind: &Bind,
    flaws: &mut Vec<Diagnostic>,
    span_table: &ast::SpanTable,
) {
    let contracts = bind
        .anonymous_result_alternatives
        .iter()
        .filter_map(|alternative| {
            alternative
                .value
                .proposition
                .as_ref()
                .map(|proposition| (alternative.value.label, proposition))
        })
        .collect::<HashMap<_, _>>();
    if contracts.is_empty() || matches!(bind.value, BindValue::Unassigned) {
        return;
    }
    enum Work<'a> {
        Sequence(
            &'a [Typed<Expr>],
            &'a [Typed<Expr>],
            Vec<ast::ProofProposition>,
        ),
        Expression(&'a Typed<Expr>, Vec<ast::ProofProposition>, bool),
    }
    let mut pending = match &bind.value {
        BindValue::Expr(expr) => vec![Work::Expression(expr, Vec::new(), true)],
        BindValue::Body { exprs, ret } => {
            vec![Work::Sequence(exprs, &ret.values, Vec::new())]
        }
        BindValue::Unassigned => return,
    };
    while let Some(work) = pending.pop() {
        match work {
            Work::Sequence(statements, exits, mut facts) => {
                let mut terminated = false;
                for expression in statements {
                    pending.push(Work::Expression(expression, facts.clone(), false));
                    if let Expr::Bind(local) = &expression.value
                        && local.params.is_none()
                        && let Some(refinement) = &local.return_refinement
                    {
                        facts.extend(proof_facts_for_refinement(refinement, local.name));
                    }
                    if let Expr::When(branch) = &expression.value {
                        for arm in &branch.arms {
                            if let ast::WhenArm::Cond {
                                condition, body, ..
                            } = arm
                                && surface_expression_returns(&body.value)
                            {
                                facts.extend(proof_facts_for_condition(condition, false));
                            } else {
                                break;
                            }
                        }
                    }
                    if surface_expression_returns(&expression.value) {
                        terminated = true;
                        break;
                    }
                }
                if !terminated {
                    for exit in exits {
                        pending.push(Work::Expression(exit, facts.clone(), true));
                    }
                }
            }
            Work::Expression(expression, mut facts, value) => match &expression.value {
                Expr::Return(ret) => {
                    for result in &ret.values {
                        pending.push(Work::Expression(result, facts.clone(), true));
                    }
                }
                Expr::Block(items) => {
                    let (statements, exits) = if value && !items.is_empty() {
                        items.split_at(items.len() - 1)
                    } else {
                        (items.as_slice(), &[][..])
                    };
                    pending.push(Work::Sequence(statements, exits, facts));
                }
                Expr::Bind(local) if local.params.is_none() => match &local.value {
                    BindValue::Expr(body) => {
                        pending.push(Work::Expression(body, facts, false));
                    }
                    BindValue::Body { exprs, ret } => {
                        pending.push(Work::Sequence(exprs, &ret.values, facts));
                    }
                    BindValue::Unassigned => {}
                },
                Expr::Bind(_) => {}
                Expr::When(branch) => {
                    for arm in &branch.arms {
                        let (body, condition) = match arm {
                            ast::WhenArm::Cond {
                                condition, body, ..
                            } => (body, Some(condition.clone())),
                            ast::WhenArm::Is { pattern, body, .. } => (
                                body,
                                branch.subject.as_ref().map(|subject| ast::Condition::Is {
                                    subject: subject.clone(),
                                    pattern: pattern.clone(),
                                }),
                            ),
                            ast::WhenArm::Else(body, _) => (body, None),
                        };
                        let mut taken = facts.clone();
                        if let Some(condition) = condition {
                            taken.extend(proof_facts_for_condition(&condition, true));
                            facts.extend(proof_facts_for_condition(&condition, false));
                        }
                        pending.push(Work::Expression(body, taken, value));
                    }
                }
                Expr::Loop(ast::Loop::While(body)) => {
                    facts.extend(proof_facts_for_condition(&body.condition, true));
                    pending.push(Work::Sequence(&body.exprs, &[], facts));
                }
                Expr::Loop(ast::Loop::ForIn(body)) => {
                    pending.push(Work::Sequence(&body.exprs, &[], facts));
                }
                _ if value => {
                    verify_result_alternative(expression, &facts, &contracts, flaws, span_table);
                }
                _ => {}
            },
        }
    }
}

fn surface_expression_returns(expression: &Expr) -> bool {
    match expression {
        Expr::Return(_) => true,
        Expr::Block(items) => items
            .iter()
            .any(|item| surface_expression_returns(&item.value)),
        Expr::Bind(bind) if bind.params.is_none() => match &bind.value {
            BindValue::Body { .. } => true,
            BindValue::Expr(value) => surface_expression_returns(&value.value),
            BindValue::Unassigned => false,
        },
        Expr::When(branch) => {
            branch
                .arms
                .iter()
                .any(|arm| matches!(arm, ast::WhenArm::Else(..)))
                && branch.arms.iter().all(|arm| {
                    let body = match arm {
                        ast::WhenArm::Cond { body, .. }
                        | ast::WhenArm::Is { body, .. }
                        | ast::WhenArm::Else(body, _) => body,
                    };
                    surface_expression_returns(&body.value)
                })
        }
        _ => false,
    }
}

fn verify_result_alternative(
    exit: &Typed<Expr>,
    facts: &[ast::ProofProposition],
    contracts: &HashMap<Intern<String>, &ast::ProofProposition>,
    flaws: &mut Vec<Diagnostic>,
    span_table: &ast::SpanTable,
) {
    let label = match &exit.value {
        Expr::AnonymousTag(label) => *label,
        Expr::TagCall(call) if call.args.is_empty() && call.qual_path.is_none() => call.name,
        _ => return,
    };
    let Some(contract) = contracts.get(&label) else {
        return;
    };
    let outcome = if proof_is_implied(contract, facts) {
        None
    } else if proof_complement(contract)
        .as_ref()
        .is_some_and(|complement| proof_is_implied(complement, facts))
    {
        Some((
            // type-result-proof-failed
            DiagnosticCode::Type(TypeCode::Result(ResultCode::ProofFailed)),
            format!("construction of `{label}` contradicts its `thus` contract `{contract}`"),
        ))
    } else {
        Some((
            // type-result-proof-unproven
            DiagnosticCode::Type(TypeCode::Result(ResultCode::ProofUnproven)),
            format!("construction of `{label}` does not prove its `thus` contract `{contract}`"),
        ))
    };
    if let Some((code, message)) = outcome {
        flaws.push(
            Diagnostic::new(code, message)
                .with_arg("alternative", label.as_str().to_string())
                .at_span_id(exit.span_id, span_table),
        );
    }
}

fn proof_is_implied(proposition: &ast::ProofProposition, facts: &[ast::ProofProposition]) -> bool {
    let mut leaves = Vec::new();
    let mut pending = facts.iter().collect::<Vec<_>>();
    while let Some(fact) = pending.pop() {
        if let ast::ProofProposition::And(left, right) = fact {
            pending.extend([left.as_ref(), right.as_ref()]);
        } else {
            leaves.push(fact);
        }
    }
    if leaves
        .iter()
        .any(|fact| proof_propositions_equal(proposition, fact, &leaves))
    {
        return true;
    }
    match proposition {
        ast::ProofProposition::Compare {
            left,
            relation: ast::ProofRelation::Equal,
            right,
        } => proof_terms_equal(left, right, &leaves),
        ast::ProofProposition::And(left, right) => {
            proof_is_implied(left, facts) && proof_is_implied(right, facts)
        }
        ast::ProofProposition::Or(left, right) => {
            proof_is_implied(left, facts) || proof_is_implied(right, facts)
        }
        ast::ProofProposition::Not(inner) => facts
            .iter()
            .any(|fact| proof_complement(fact).as_ref() == Some(inner.as_ref())),
        _ => false,
    }
}

fn proof_propositions_equal(
    left: &ast::ProofProposition,
    right: &ast::ProofProposition,
    facts: &[&ast::ProofProposition],
) -> bool {
    match (left, right) {
        (
            ast::ProofProposition::Compare {
                left: a,
                relation: ar,
                right: b,
            },
            ast::ProofProposition::Compare {
                left: c,
                relation: cr,
                right: d,
            },
        ) => ar == cr && proof_terms_equal(a, c, facts) && proof_terms_equal(b, d, facts),
        _ => left == right,
    }
}

fn proof_terms_equal(
    left: &ast::ProofTerm,
    right: &ast::ProofTerm,
    facts: &[&ast::ProofProposition],
) -> bool {
    use ast::ProofTerm;
    if left == right {
        return true;
    }
    match (left, right) {
        (ProofTerm::Name(left), ProofTerm::Name(right)) => {
            let mut pending = vec![*left];
            let mut seen = HashSet::new();
            while let Some(name) = pending.pop() {
                if name == *right {
                    return true;
                }
                if !seen.insert(name) {
                    continue;
                }
                for fact in facts {
                    if let ast::ProofProposition::Compare {
                        left: ProofTerm::Name(a),
                        relation: ast::ProofRelation::Equal,
                        right: ProofTerm::Name(b),
                    } = fact
                    {
                        if *a == name {
                            pending.push(*b);
                        }
                        if *b == name {
                            pending.push(*a);
                        }
                    }
                }
            }
            false
        }
        (ProofTerm::Add(a, b), ProofTerm::Add(c, d))
        | (ProofTerm::Sub(a, b), ProofTerm::Sub(c, d))
        | (ProofTerm::Mul(a, b), ProofTerm::Mul(c, d))
        | (ProofTerm::Remainder(a, b), ProofTerm::Remainder(c, d)) => {
            proof_terms_equal(a, c, facts) && proof_terms_equal(b, d, facts)
        }
        (ProofTerm::PowerOfTwo(a), ProofTerm::PowerOfTwo(b)) => proof_terms_equal(a, b, facts),
        _ => false,
    }
}

fn proof_facts_for_refinement(
    refinement: &ast::PredicateExpr,
    name: Intern<String>,
) -> Vec<ast::ProofProposition> {
    use ast::{PredicateExpr, ProofProposition, ProofRelation, ProofTerm};
    let (relation, right) = match refinement {
        PredicateExpr::Lt(right) => (ProofRelation::Less, right),
        PredicateExpr::Gt(right) => (ProofRelation::Greater, right),
        PredicateExpr::Le(right) => (ProofRelation::LessOrEqual, right),
        PredicateExpr::Ge(right) => (ProofRelation::GreaterOrEqual, right),
        PredicateExpr::Eq(right) => (ProofRelation::Equal, right),
        PredicateExpr::Ne(right) => (ProofRelation::NotEqual, right),
        PredicateExpr::And(children) => {
            return children
                .iter()
                .flat_map(|child| proof_facts_for_refinement(child, name))
                .collect();
        }
        PredicateExpr::Proposition(proposition) => return vec![proposition.as_ref().clone()],
    };
    proof_term_for_normal(right)
        .map(|right| ProofProposition::Compare {
            left: ProofTerm::Name(name),
            relation,
            right,
        })
        .into_iter()
        .collect()
}

fn proof_term_for_normal(expr: &ast::NormalExpr) -> Option<ast::ProofTerm> {
    use ast::{NormalExpr, ProofTerm};
    match expr {
        NormalExpr::Var(name) => Some(ProofTerm::Name(*name)),
        NormalExpr::Value(ast::ConstValue::Int(value)) => Some(ProofTerm::Value(*value)),
        NormalExpr::Add(left, right) => Some(ProofTerm::Add(
            Box::new(proof_term_for_normal(left)?),
            Box::new(proof_term_for_normal(right)?),
        )),
        NormalExpr::Sub(left, right) => Some(ProofTerm::Sub(
            Box::new(proof_term_for_normal(left)?),
            Box::new(proof_term_for_normal(right)?),
        )),
        NormalExpr::Mul(left, right) => Some(ProofTerm::Mul(
            Box::new(proof_term_for_normal(left)?),
            Box::new(proof_term_for_normal(right)?),
        )),
        _ => None,
    }
}

fn proof_facts_for_condition(
    condition: &ast::Condition,
    taken: bool,
) -> Vec<ast::ProofProposition> {
    match condition {
        ast::Condition::Proposition { subject } => proof_for_comparison_expr(&subject.value)
            .and_then(|proof| {
                if taken {
                    Some(proof)
                } else {
                    proof_complement(&proof)
                }
            })
            .into_iter()
            .collect(),
        ast::Condition::Compare {
            left,
            relation,
            right,
        } => {
            let Some(left) = proof_term_for_expr(&left.value) else {
                return Vec::new();
            };
            let Some(right) = proof_term_for_expr(&right.value) else {
                return Vec::new();
            };
            vec![ast::ProofProposition::Compare {
                left,
                relation: if taken {
                    *relation
                } else {
                    relation.complement()
                },
                right,
            }]
        }
        ast::Condition::Is { subject, pattern } => {
            if taken
                && let ast::Pattern::InRange { bounds, .. } = &pattern.value
                && let Some(value) = proof_term_for_expr(&subject.value)
            {
                let (start, end) = match bounds {
                    ast::InRangeBounds::Literal(start, end) => {
                        (ast::ProofTerm::Value(*start), ast::ProofTerm::Value(*end))
                    }
                    ast::InRangeBounds::LiteralToTag(start, end) => {
                        (ast::ProofTerm::Value(*start), ast::ProofTerm::Name(*end))
                    }
                    ast::InRangeBounds::Tag(_) => return Vec::new(),
                };
                return vec![
                    ast::ProofProposition::Compare {
                        left: value.clone(),
                        relation: ast::ProofRelation::GreaterOrEqual,
                        right: start,
                    },
                    ast::ProofProposition::Compare {
                        left: value,
                        relation: ast::ProofRelation::LessOrEqual,
                        right: end,
                    },
                ];
            }
            Vec::new()
        }
        ast::Condition::Not(inner) => proof_facts_for_condition(inner, !taken),
        ast::Condition::And(left, right) if taken => {
            let mut facts = proof_facts_for_condition(left, true);
            facts.extend(proof_facts_for_condition(right, true));
            facts
        }
        ast::Condition::Or(left, right) if !taken => {
            let mut facts = proof_facts_for_condition(left, false);
            facts.extend(proof_facts_for_condition(right, false));
            facts
        }
        ast::Condition::And(_, _) | ast::Condition::Or(_, _) => Vec::new(),
    }
}

fn proof_for_comparison_expr(expr: &Expr) -> Option<ast::ProofProposition> {
    let Expr::Binary(binary) = expr else {
        return None;
    };
    let relation = ast::ProofRelation::for_bin_op(&binary.op)?;
    Some(ast::ProofProposition::Compare {
        left: proof_term_for_expr(&binary.lhs.value)?,
        relation,
        right: proof_term_for_expr(&binary.rhs.value)?,
    })
}

fn proof_term_for_expr(expr: &Expr) -> Option<ast::ProofTerm> {
    match expr {
        Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => {
            Some(ast::ProofTerm::Name(call.path.root))
        }
        Expr::SelfRef => Some(ast::ProofTerm::Name(Intern::from_ref("self"))),
        Expr::Lit(Literal::Int(value)) => Some(ast::ProofTerm::Value(*value)),
        Expr::Binary(binary) => {
            let left = Box::new(proof_term_for_expr(&binary.lhs.value)?);
            let right = Box::new(proof_term_for_expr(&binary.rhs.value)?);
            match binary.op {
                ast::BinOp::Add => Some(ast::ProofTerm::Add(left, right)),
                ast::BinOp::Subtract => Some(ast::ProofTerm::Sub(left, right)),
                ast::BinOp::Multiply => Some(ast::ProofTerm::Mul(left, right)),
                ast::BinOp::Modulo => Some(ast::ProofTerm::Remainder(left, right)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn proof_complement(proposition: &ast::ProofProposition) -> Option<ast::ProofProposition> {
    let ast::ProofProposition::Compare {
        left,
        relation,
        right,
    } = proposition
    else {
        return None;
    };
    let relation = relation.complement();
    Some(ast::ProofProposition::Compare {
        left: left.clone(),
        relation,
        right: right.clone(),
    })
}

fn declaration_depends_on_target(
    declaration: &ast::Declare,
    target_dependent_names: &HashSet<Intern<String>>,
) -> bool {
    if declaration
        .attributes
        .bits
        .as_ref()
        .is_some_and(|bits| expr_references_target_dependency(&bits.value, target_dependent_names))
    {
        return true;
    }
    let ast::DeclareValue::Refinement(predicate) = &declaration.value else {
        return false;
    };
    let mut names = HashSet::new();
    let _ = ProofNameCollector(&mut names).visit_predicate(predicate);
    names
        .iter()
        .any(|name| target_dependent_names.contains(name))
}

fn expr_references_target_dependency(
    expr: &ast::Expr,
    target_dependent_names: &HashSet<Intern<String>>,
) -> bool {
    matches!(expr, ast::Expr::TargetQuery { .. })
        || matches!(
            expr,
            ast::Expr::FnCall(call)
                if call.args.is_none()
                    && target_dependent_names.contains(&Intern::new(call.path.value.to_qualified_string()))
        )
}

struct ProofNameCollector<'a>(&'a mut HashSet<Intern<String>>);

impl ProofVisitor for ProofNameCollector<'_> {
    fn visit_normal(&mut self, expr: &ast::NormalExpr) -> ControlFlow<()> {
        if let ast::NormalExpr::Var(name) = expr {
            self.0.insert(*name);
        }
        walk_normal(self, expr)
    }

    fn visit_term(&mut self, term: &ast::ProofTerm) -> ControlFlow<()> {
        if let ast::ProofTerm::Name(name) = term {
            self.0.insert(*name);
        }
        walk_term(self, term)
    }

    fn visit_reference(&mut self, reference: &ast::TypeReference) -> ControlFlow<()> {
        if let Some(name) = reference.head_name() {
            self.0.insert(name);
        }
        walk_reference(self, reference)
    }
}

fn anonymous_result_labels(file_ast: &FileAst) -> HashSet<Intern<String>> {
    file_ast
        .defs
        .values()
        .flat_map(|bind| {
            bind.anonymous_result_alternatives
                .iter()
                .map(|alternative| alternative.value.label)
                .chain(inferred_result_alternatives(
                    &bind.value,
                    (&file_ast.tags).into(),
                ))
        })
        .collect()
}

fn contains_unresolved_result_label(ty: &Ty, labels: &HashSet<Intern<String>>) -> bool {
    match ty {
        Ty::Opaque(name) => labels.contains(name),
        Ty::Named { .. } | Ty::Array { .. } | Ty::Ptr { .. } | Ty::OwnedStorage { .. } | Ty::Address { .. } | Ty::Ref { .. } => {
            ty.any_child(|child| child.ty().is_some_and(|ty| contains_unresolved_result_label(ty, labels)))
        }
        Ty::Record { .. } | Ty::Union { .. } => ty.any_child(|child| {
            matches!(child, TyChild::Type(ty) if contains_unresolved_result_label(ty, labels))
        }),
        _ => false,
    }
}

fn anonymous_result_annotation_diagnostic(
    span: ast::SpanId,
    span_table: &ast::SpanTable,
) -> Diagnostic {
    // type-anonymous-result-not-nameable
    Diagnostic::new(
        ResultCode::AnonymousNotNameable,
        "an anonymous result-family type cannot be named in a source annotation",
    )
    .with_help("use inference, transparent forwarding, or translate to a named union")
    .at_span_id(span, span_table)
}

pub(super) fn resolve_param_types(
    bind: &Bind,
    registry: &crate::TypeRegistry,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: TagDeclarations<'_>,
    dependent_binder: BinderId,
    typevar_subst: &HashMap<Intern<String>, Ty>,
) -> Vec<(Intern<String>, Ty)> {
    let Some(params) = &bind.params else {
        return Vec::new();
    };
    params
        .iter()
        .map(|(name, kind)| {
            let ty = match &kind.kind {
                ParameterKind::Tagged(sp) => TypeEnv::new(tag_types)
                    .with_registry(registry)
                    .with_subst(typevar_subst)
                    .with_tag_params(tag_params)
                    .with_tag_declaration_lookup(tag_decls)
                    .with_dependent_binder(dependent_binder)
                    .resolve_expr(&sp.value),
                ParameterKind::ValueParam { ty } | ParameterKind::Inferred { ty } => {
                    TypeEnv::new(tag_types)
                        .with_registry(registry)
                        .with_subst(typevar_subst)
                        .with_tag_params(tag_params)
                        .with_tag_declaration_lookup(tag_decls)
                        .with_dependent_binder(dependent_binder)
                        .resolve_expr(&ty.value)
                }
                ParameterKind::Generic if name.as_str() == "self" && bind.is_method() => {
                    let receiver = tag_types
                        .get(&Intern::from_ref("Self"))
                        .cloned()
                        .unwrap_or(Ty::Opaque(Intern::from_ref("Self")));
                    match bind.param_conventions.get(name) {
                        Some(ParamConvention::Observe) => Ty::Ref {
                            inner: Box::new(receiver),
                            mutable: false,
                        },
                        Some(ParamConvention::Mutate) => Ty::Ref {
                            inner: Box::new(receiver),
                            mutable: true,
                        },
                        _ => receiver,
                    }
                }
                ParameterKind::Generic if super::parameters::is_inferred_value(bind, *name) => {
                    super::parameters::inferred_value_type(bind, *name)
                }
                ParameterKind::Proposition(predicate) => Ty::Proposition(predicate.clone()),
                ParameterKind::Generic => bind
                    .param_refinements
                    .get(name)
                    .map(|predicate| Ty::AnonymousInteger {
                        validity: ast::integer::IntegerValidity::new(
                            ast::integer::IntegerDomain::from_named_refinement(predicate, *name),
                        ),
                    })
                    .unwrap_or(Ty::Opaque(*name)),
                ParameterKind::Default(expr) => {
                    let env = TyInferEnv {
                        tag_types,
                        fn_return_types: &HashMap::new(),
                        locals: &HashMap::new(),
                        tag_params: Some(tag_params),
                    };
                    expr.infer_ty(&env)
                }
            };
            let ty = match bind.param_conventions.get(name) {
                Some(convention @ (ParamConvention::Observe | ParamConvention::Mutate))
                    if !ty.is_ref() =>
                {
                    Ty::Ref {
                        inner: Box::new(ty),
                        mutable: *convention == ParamConvention::Mutate,
                    }
                }
                _ => ty,
            };
            (*name, ty)
        })
        .collect()
}

fn field_refinements(declare: &ast::Declare) -> HashMap<Intern<String>, ast::PredicateExpr> {
    let ast::DeclareValue::Has(members) = &declare.value else {
        return HashMap::new();
    };
    members
        .iter()
        .filter_map(|member| match member {
            ast::HasMember::Property(property)
                if property
                    .qualifier
                    .as_ref()
                    .is_none_or(|qualifier| qualifier.name == declare.name) =>
            {
                property
                    .refinement
                    .as_ref()
                    .map(|refinement| (property.name, refinement.clone()))
            }
            _ => None,
        })
        .collect()
}

type PrivateMemberGrants = HashMap<Intern<String>, Vec<Vec<Intern<String>>>>;
type PrivateFieldNames = HashSet<Intern<String>>;

fn member_visibility(declare: &ast::Declare) -> (PrivateMemberGrants, PrivateFieldNames) {
    let mut private_members = HashMap::new();
    let mut private_fields = HashSet::new();
    if let ast::DeclareValue::Has(members) = &declare.value {
        for member in members {
            let (name, attributes, stored) = match member {
                ast::HasMember::Property(field) => (
                    field.name,
                    &field.attributes,
                    field
                        .qualifier
                        .as_ref()
                        .is_none_or(|qualifier| qualifier.name == declare.name),
                ),
                ast::HasMember::Function(method) => (method.name, &method.attributes, false),
            };
            if attributes.is_private() {
                private_members.insert(name, private_grants(attributes));
                if stored {
                    private_fields.insert(name);
                }
            }
        }
    }
    (private_members, private_fields)
}

fn private_grants(attributes: &ast::BindAttributes) -> Vec<Vec<Intern<String>>> {
    attributes
        .raw_attributes
        .iter()
        .flatten()
        .filter_map(|attribute| match attribute {
            ast::AttributeItem::Call { name, args, .. } if name.as_str() == "private" => Some(args),
            _ => None,
        })
        .flatten()
        .filter_map(|argument| {
            if let ast::Expr::FnCall(call) = &argument.value
                && call.args.is_none()
            {
                let mut module = vec![call.path.value.root];
                module.extend(call.path.value.segments.iter().copied());
                Some(module)
            } else {
                None
            }
        })
        .collect()
}

fn validated_sequence_literal_acceptance(
    declare: &ast::Declare,
) -> Option<crate::SequenceLiteralAcceptance> {
    let params = declare.params.as_ref()?;
    let element_is_type = |element_parameter: Intern<String>| {
        params
            .get(&element_parameter)
            .is_some_and(|parameter| matches!(parameter.kind, ParameterKind::Generic))
    };
    if let Some(contract) = declare.attributes.fixed_array.as_ref() {
        if contract.element_parameter == contract.length_parameter {
            return None;
        }
        let length_is_const = params
            .get(&contract.length_parameter)
            .is_some_and(|parameter| {
                matches!(
                    parameter.kind,
                    ParameterKind::Tagged(_)
                        | ParameterKind::ValueParam { .. }
                        | ParameterKind::Inferred { .. }
                        | ParameterKind::Default(_)
                )
            });
        return (element_is_type(contract.element_parameter) && length_is_const).then_some(
            crate::SequenceLiteralAcceptance {
                element_parameter: contract.element_parameter,
                materialization: crate::SequenceLiteralMaterialization::FixedArray {
                    length_parameter: contract.length_parameter,
                },
            },
        );
    }
    let contract = declare.attributes.sequence_builder.as_ref()?;
    params
        .contains_key(&contract.element_parameter)
        .then_some(crate::SequenceLiteralAcceptance {
            element_parameter: contract.element_parameter,
            materialization: crate::SequenceLiteralMaterialization::OwnedBuilder {
                empty_member: contract.empty_member,
                push_member: contract.push_member,
            },
        })
}

fn param_kind_from_parameter_kind(
    kind: &ParameterKind,
    tag_types: &HashMap<Intern<String>, Ty>,
) -> ParamKind {
    match kind {
        ParameterKind::Proposition(predicate) => {
            ParamKind::Value(Box::new(Ty::Proposition(predicate.clone())))
        }
        ParameterKind::Generic => ParamKind::Type,
        ParameterKind::Tagged(sp) => {
            let ty = TypeEnv::new(tag_types).resolve_expr(&sp.value);
            ParamKind::Value(Box::new(ty))
        }
        ParameterKind::ValueParam { ty } | ParameterKind::Inferred { ty } => {
            let ty = TypeEnv::new(tag_types).resolve_expr(&ty.value);
            ParamKind::Value(Box::new(ty))
        }
        ParameterKind::Default(expr) if expr_is_type_surface(&expr.value) => ParamKind::Type,
        ParameterKind::Default(expr) => {
            let env = TyInferEnv {
                tag_types,
                fn_return_types: &HashMap::new(),
                locals: &HashMap::new(),
                tag_params: None,
            };
            ParamKind::Value(Box::new(expr.infer_ty(&env)))
        }
    }
}

fn is_type_parameter(kind: &ParameterKind) -> bool {
    match kind {
        ParameterKind::Proposition(_) => false,
        ParameterKind::Generic => true,
        ParameterKind::Tagged(_)
        | ParameterKind::ValueParam { .. }
        | ParameterKind::Inferred { .. } => false,
        ParameterKind::Default(expr) => expr_is_type_surface(&expr.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::ty::{NamedTypeInstance, SequenceTypeElement, TypeId};

    #[test]
    fn target_group_paths_reject_missing_nominal_definitions() {
        let name = Intern::from_ref("Missing");
        let ty = Ty::Named {
            instance: NamedTypeInstance::new(TypeId { file: 7, name }),
            name,
        };
        let mut path = ast::GroupPath::root(Intern::from_ref("group"));
        path.segments.push(Intern::from_ref("field"));
        assert!(matches!(
            resolve_group_path(&ty, &path, &crate::TypeRegistry::default()),
            Err(GroupPathError::InvalidProjection { .. })
        ));
    }

    #[test]
    fn target_group_paths_reject_recursive_nominal_definitions() {
        let parsed = parser::parse_from_str("Node has\n    next ref Node\n");
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let typed = stage_declare(&parsed, FileId(7), &TransformCtx::new());
        let node = &typed.tags[&TagId(Intern::from_ref("Node"))].resolved_ty;
        let id = node.type_id().expect("nominal Node");
        let mut path = ast::GroupPath::root(Intern::from_ref("group"));
        path.segments.push(Intern::from_ref("next"));
        for definition in [
            node.clone(),
            Ty::Ref {
                inner: Box::new(node.clone()),
                mutable: false,
            },
        ] {
            let mut registry = typed.type_registry.clone();
            let mut semantics = registry.declaration(id).expect("Node declaration").clone();
            semantics.definition = definition;
            registry.insert(id, semantics);
            assert!(matches!(
                resolve_group_path(node, &path, &registry),
                Err(GroupPathError::InvalidProjection { .. })
            ));
        }
    }

    #[test]
    fn target_group_paths_unwrap_finite_reference_prefixes() {
        let parsed = parser::parse_from_str("Node has\n    next ref Node\n");
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let typed = stage_declare(&parsed, FileId(7), &TransformCtx::new());
        let node = &typed.tags[&TagId(Intern::from_ref("Node"))].resolved_ty;
        let field = Intern::from_ref("next");
        let mut path = ast::GroupPath::root(Intern::from_ref("group"));
        path.segments.push(field);
        let reference = Ty::Ref {
            inner: Box::new(node.clone()),
            mutable: false,
        };
        let root = Ty::Ref {
            inner: Box::new(reference.clone()),
            mutable: true,
        };
        assert_eq!(
            resolve_group_path(&root, &path, &typed.type_registry),
            Ok((
                vec![GroupProjection::Field {
                    name: field,
                    index: 0
                }],
                typed.type_registry.resolved_definition_for_type(node)
            ))
        );
    }

    #[test]
    fn stored_sequence_references_resolve_to_their_enclosing_nominal_type() {
        let parsed = parser::parse_from_str("Node has\n    next [ref Node, mut Node]\n");
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let typed = stage_declare(&parsed, FileId(7), &TransformCtx::new());
        assert!(
            typed.declaration_flaws.is_empty(),
            "{:?}",
            typed.declaration_flaws
        );
        let node = &typed.tags[&TagId(Intern::from_ref("Node"))].resolved_ty;
        let Ty::Record { fields, .. } = typed.type_registry.resolved_definition_for_type(node)
        else {
            panic!("Node record");
        };
        assert_eq!(
            fields,
            vec![(
                Intern::from_ref("next"),
                Box::new(Ty::Sequence(vec![
                    SequenceTypeElement {
                        name: None,
                        ty: Box::new(Ty::Ref {
                            inner: Box::new(node.clone()),
                            mutable: false
                        })
                    },
                    SequenceTypeElement {
                        name: None,
                        ty: Box::new(Ty::Ref {
                            inner: Box::new(node.clone()),
                            mutable: true
                        })
                    },
                ]))
            )]
        );
    }

    #[test]
    fn sequence_self_references_preserve_the_declaration_instance() {
        let name = Intern::from_ref("Node");
        let field = Intern::from_ref("next");
        let unrelated = Ty::Opaque(Intern::from_ref("Other"));
        let instance = NamedTypeInstance::new(TypeId { file: 7, name }).with_arguments(vec![(
            Intern::from_ref("element"),
            ast::TyArg::Type(Box::new(unrelated.clone())),
        )]);
        let ty = Ty::Sequence(vec![
            SequenceTypeElement {
                name: Some(field),
                ty: Box::new(Ty::Ref {
                    inner: Box::new(Ty::Opaque(name)),
                    mutable: true,
                }),
            },
            SequenceTypeElement {
                name: None,
                ty: Box::new(unrelated.clone()),
            },
        ]);
        let expected = Ty::Sequence(vec![
            SequenceTypeElement {
                name: Some(field),
                ty: Box::new(Ty::Ref {
                    inner: Box::new(Ty::Named {
                        instance: instance.clone(),
                        name,
                    }),
                    mutable: true,
                }),
            },
            SequenceTypeElement {
                name: None,
                ty: Box::new(unrelated),
            },
        ]);
        assert_eq!(
            preserve_self_reference_with_instance(&ty, name, &instance),
            expected
        );
    }

    #[test]
    fn nominal_and_anonymous_label_queries_keep_distinct_traversal_rules() {
        let label = Intern::from_ref("Label");
        let name = Intern::from_ref("Container");
        let opaque = Ty::Opaque(label);
        let arguments = vec![(name, ast::TyArg::Type(Box::new(opaque.clone())))];
        let cases = [
            (
                Ty::Named {
                    name,
                    instance: NamedTypeInstance::new(TypeId { file: 0, name })
                        .with_arguments(arguments.clone()),
                },
                true,
            ),
            (
                Ty::Record {
                    name,
                    fields: vec![],
                    resolved_params: Some(arguments.clone()),
                },
                false,
            ),
            (
                Ty::Union {
                    name,
                    variants: vec![],
                    literal_values: None,
                    resolved_params: Some(arguments),
                },
                false,
            ),
            (
                Ty::Record {
                    name,
                    fields: vec![(name, Box::new(opaque.clone()))],
                    resolved_params: None,
                },
                true,
            ),
            (
                Ty::Sequence(vec![SequenceTypeElement {
                    name: Some(name),
                    ty: Box::new(opaque.clone()),
                }]),
                false,
            ),
            (
                Ty::UnresolvedSequence {
                    element: Some(Box::new(opaque)),
                    length: 1.into(),
                },
                false,
            ),
        ];
        for (ty, contains_label) in cases {
            assert!(references_nominal_type(&ty, label), "{ty:?}");
            assert_eq!(
                contains_unresolved_result_label(&ty, &HashSet::from([label])),
                contains_label,
                "{ty:?}"
            );
            assert!(!references_nominal_type(&ty, Intern::from_ref("Absent")));
        }
    }
}

fn return_contract_matches(registry: &crate::TypeRegistry, actual: &Ty, expected: &Ty) -> bool {
    actual == expected || super::return_types::same_union_layout(registry, actual, expected)
}

fn validate_payload_names(
    method: &ast::HasFunction,
    flaws: &mut Vec<(SpanId, Diagnostic)>,
    spans: &ast::SpanTable,
) {
    for proof in &method.payload_postconditions {
        let surface = ast::PredicateExpr::Proposition(Box::new(proof.proposition.clone()));
        for name in super::refinement_terms::predicate_names(&surface) {
            let root = Intern::from_ref(name.as_str().split('.').next().unwrap_or(name.as_str()));
            if root != proof.name && !method.params.contains_key(&root) {
                flaws.push((
                    method.name_span,
                    // type-constraint-unknown-name
                    Diagnostic::new(
                        ProofCode::UnknownName,
                        format!("payload contract refers to unknown value `{root}`"),
                    )
                    .at_span_id(method.name_span, spans),
                ));
            }
        }
    }
}
