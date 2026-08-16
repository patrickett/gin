//! Stage 1: Declare — Resolve tag/bind declarations, build file-level index.
//!
//! This stage walks the `FileAst` tags and defs, translates them into
//! [`TypedTag`] and [`TypedBind`], populates `tag_types`, `fn_return_types`,
//! and `variant_map`, and detects declaration-level flaws.

use internment::Intern;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Enough for mutual refs (e.g. `Type` ↔ `NamedTy`); keep low — each pass re-resolves every tag.
const TAG_RESOLVE_MAX_PASSES: usize = 8;

use diagnostic::Diagnostic;

use super::TransformCtx;
use super::lower_ty::nominalize_explicit_ty_surface;
use crate::analysis::expr_is_type_surface;
use crate::analysis::{TyInfer, TyInferEnv, TypeEnv};
use crate::compile_time_trait::CompileTimeTraitRegistry;
use crate::compile_time_trait::{
    RESERVED_TRAITS, ReflectionContractIssue, synthesize_reflectable_trait,
};
use crate::normal_expr::Normalize;
use crate::representation::RepresentationError;
use crate::ty::{Ty, UnionVariant, reference_semantics_for_type};
use crate::type_registry::ReferenceSemantics;
use crate::typed::{
    Availability, AvailabilityRequirement, CallCapability, DefId, ExprId, FileId, GroupId,
    GroupProjection, TagId, TypedBind, TypedFileAst, TypedGroup, TypedTag, VariantMap,
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
    let validity = ty.anonymous_integer_validity()?;
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
            let natural = if let Some(ast::ConstValue::Int(value)) = bits.const_value.as_ref() {
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
                    ast::Expr::FnCall(call)
                        if call.args.is_none() && call.path.value.segments.is_empty() =>
                    {
                        ast::integer::CanonicalNaturalExpr::Symbolic(ast::NormalExpr::Var(
                            call.path.value.root,
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
    Some(ast::integer::NominalIntegerSemantics {
        validity: validity.clone(),
        representation_rule,
        interpretation,
    })
}

impl TypedTag {
    pub fn resolve_param_kinds(
        &self,
        tag_types: &HashMap<Intern<String>, Ty>,
    ) -> Vec<(Intern<String>, ParamKind)> {
        let Some(params) = &self.params else {
            return Vec::new();
        };
        params
            .iter()
            .map(|(name, kind)| {
                let kind = match &kind.kind {
                    ParameterKind::Generic => ParamKind::Type,
                    ParameterKind::Tagged(sp) if is_type_param_marker(&sp.value) => ParamKind::Type,
                    ParameterKind::Tagged(sp) => {
                        let ty = TypeEnv::new(tag_types).resolve_expr(&sp.value);
                        ParamKind::Value(Box::new(ty))
                    }
                    ParameterKind::ValueParam { ty } | ParameterKind::Inferred { ty } => {
                        let ty = TypeEnv::new(tag_types).resolve_expr(&ty.value);
                        ParamKind::Value(Box::new(ty))
                    }
                    ParameterKind::Default(expr) => {
                        let env = TyInferEnv {
                            tag_types,
                            fn_return_types: &HashMap::new(),
                            locals: &HashMap::new(),
                            tag_params: None,
                        };
                        ParamKind::Value(Box::new(expr.infer_ty(&env)))
                    }
                };
                (*name, kind)
            })
            .collect()
    }
}

/// Takes `&FileAst` and returns a `TypedFileAst` with declarations
/// resolved and the file-level index populated. The expression arena is left
/// empty — it will be filled in Stage 2.
pub fn stage_declare(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    let mut typed = TypedFileAst::new(file_id, file_ast.span_table.clone());
    typed.target_layout =
        crate::layout::TargetLayout::from_compile_target(&ctx.compile_target).ok();
    let anonymous_result_labels = anonymous_result_labels(file_ast);
    typed.semantic_origin = file_ast.semantic_origin.clone();
    typed.module_doc = file_ast.module_doc.clone();
    typed.type_registry.merge_from(&ctx.type_registry);
    let trait_registry =
        CompileTimeTraitRegistry::from_parse_ast(file_ast, Arc::clone(&ctx.compile_time_eval_ast));
    for declare in file_ast.tags.values() {
        let ast::DeclareValue::Has(members) = &declare.value else {
            continue;
        };
        for member in members {
            let ast::HasMember::Function(function) = member else {
                continue;
            };
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
                Some(ast::ParamConvention::Observe) => "ref self",
                Some(ast::ParamConvention::Mutate) => "mut self",
                Some(ast::ParamConvention::Consume) => "eat self",
                Some(ast::ParamConvention::Own) | None => "self",
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
            }
        }
    }
    typed.imported_trait_names = file_ast.imported_trait_names();
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
    let resolved_tags = resolve_tags_fixpoint(
        &file_ast.tags,
        &tag_params,
        &ctx.cross_file_tag_types,
        file_id,
        TAG_RESOLVE_MAX_PASSES,
    );
    let available_tags = nominal_types_for_definitions(&resolved_tags, &file_ast.tags, file_id);
    for (name, declare) in &file_ast.tags {
        let is_transparent_alias = matches!(&declare.value, ast::DeclareValue::Alias(alias)
            if !matches!(&alias.value, ast::Expr::TakePtr(_))
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
                fixed_array: validated_fixed_array_former(declare),
                reference: None,
                integer: nominal_integer_semantics(definition, &declare.attributes),
            },
        );
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
                &file_ast.tags,
                &tag_params,
                &ctx.cross_file_tag_types,
                &available_tags,
                file_id,
            )
        });
        let is_transparent_alias = matches!(&declare.value, ast::DeclareValue::Alias(alias)
            if !matches!(&alias.value, ast::Expr::TakePtr(_))
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
                Diagnostic::new(
                    "type-integer-domain-empty",
                    format!(
                        "integer domain `{start}...{end}` is empty",
                        start = start,
                        end = end
                    ),
                )
                .with_arg("min", start.to_string())
                .with_arg("max", end.to_string())
                .at_span_id(declare.name_span, &typed.span_table),
            ));
        }

        let fixed_array = validated_fixed_array_former(declare);
        let integer = nominal_integer_semantics(&definition, &declare.attributes);
        if declare.attributes.bits_span.is_some()
            && !target_dependent_names
                .as_ref()
                .is_some_and(|names| declaration_depends_on_target(declare, names))
            && let Some(integer) = &integer
            && let Err(error) = ast::integer::resolve_runtime_representation(
                &integer.validity,
                &integer.representation_rule,
            )
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
                        fixed_array,
                        reference,
                        integer,
                    },
                );
            }
            if declare.attributes.invalid_fixed_array.is_some()
                || (declare.attributes.fixed_array.is_some()
                    && typed
                        .type_registry
                        .declaration(instance)
                        .is_some_and(|declaration| declaration.fixed_array.is_none()))
            {
                typed.declaration_flaws.push((
                    declare.name_span,
                    Diagnostic::new(
                    "type-invalid-fixed-array-former",
                    "fixed-array former must name one element and one length parameter belonging to the declaration",
                    )
                    .at_span_id(declare.name_span, &typed.span_table),
                ));
            }
        }
        if resolved_ty.named_instance().is_some() {
            let has_generic_parameters = declare.params.as_ref().is_some_and(|params| {
                params
                    .values()
                    .any(|parameter| matches!(parameter.kind, ParameterKind::Generic))
            });
            if !has_generic_parameters
                && !target_dependent_names
                    .as_ref()
                    .is_some_and(|names| declaration_depends_on_target(declare, names))
            {
                let representation_type = declaration_representation_type_root(&resolved_ty);
                if declaration_has_symbolic_fixed_array_length(
                    &representation_type,
                    Some(&typed.type_registry),
                ) {
                    // Symbolic fixed array length declarations are valid in generic
                    // usage and defer concrete layout checks to monomorphic codegen paths.
                } else {
                    match crate::representation::Repr::derive(
                        &representation_type,
                        Some(&typed.type_registry),
                    ) {
                        Ok(_) => {}
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
        } else if let ast::DeclareValue::Alias(alias) = &declare.value
            && !matches!(alias.value, ast::Expr::TakePtr(_))
            && let Err(diagnostic) =
                declaration_reference_semantics(&resolved_ty, &typed.type_registry)
        {
            typed.declaration_flaws.push((
                declare.name_span,
                diagnostic.at_span_id(declare.name_span, &typed.span_table),
            ));
        }

        check_qualified_has_members(
            declare,
            file_ast,
            ctx.compile_time_eval_ast.as_ref(),
            &mut |span, diagnostic| {
                typed
                    .declaration_flaws
                    .push((span, diagnostic.at_span_id(span, &typed.span_table)));
            },
        );

        for pt in &declare.provided_traits {
            if RESERVED_TRAITS.contains(&pt.trait_name.as_str())
                || trait_registry.is_reserved_trait_name(pt.trait_name.as_str(), &typed)
            {
                typed.declaration_flaws.push((
                    pt.trait_name_span,
                    Diagnostic::new(
                        "type-reserved-trait-impl",
                        format!(
                            "trait `{}` is reserved and cannot be implemented explicitly",
                            pt.trait_name.as_str()
                        ),
                    )
                    .with_arg("trait_name", pt.trait_name.as_str().to_string()),
                ));
            }
        }

        let mut provided_traits = declare.provided_traits.clone();
        provided_traits
            .retain(|pt| !trait_registry.is_reserved_trait_name(pt.trait_name.as_str(), &typed));

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
                    let expr_id = ExprId(typed.exprs.kind.len() as u32);
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
                    typed
                        .exprs
                        .kind
                        .push(crate::typed::TypedExprKind::Lit(ast::Literal::Number(0)));
                    typed.exprs.ty.push(expr_ty);
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
                    let span = typed.span_table.get(expr.span_id);
                    typed
                        .span_to_expr
                        .entry(span.start_u32())
                        .or_insert(expr_id);
                }
            }
        }

        // Reflectable.shape materializes the whole `Type` graph; skip for IDE package transforms.
        let has_reflection_member = declare
            .provided_traits
            .iter()
            .any(|pt| trait_registry.is_reflection_trait_name(pt.trait_name.as_str(), &typed));
        if !ctx.ide_package && !has_reflection_member {
            provided_traits.extend(synthesize_reflectable_trait(
                &resolved_ty,
                &typed,
                &trait_registry,
                &typed.type_registry,
            ));
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
                                    if let Some(conv) = f.conventions.get(param_name) {
                                        match conv {
                                            ast::ParamConvention::Observe => {
                                                ty_surface.push_str("ref ")
                                            }
                                            ast::ParamConvention::Mutate => {
                                                ty_surface.push_str("mut ")
                                            }
                                            ast::ParamConvention::Consume => {
                                                ty_surface.push_str("eat ")
                                            }
                                            ast::ParamConvention::Own => {}
                                        }
                                    }
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
                                        ast::ParameterKind::Generic => {}
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
                            if let Some(et) = &f.error_ty {
                                ty_surface.push_str(" or ");
                                ty_surface.push_str(&et.value.format_surface());
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

    for bind in file_ast.defs.values() {
        check_params_for_in_range_on_bounded_tag(
            bind.params.as_ref(),
            &typed.tag_types,
            &mut typed.declaration_flaws,
        );
    }

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

    // 3. Merge cross-file variant map entries (not spliced into own to avoid O(n²)).
    for (variant_name, entries) in &ctx.cross_file_variant_map {
        typed
            .variant_map
            .entry(*variant_name)
            .or_default()
            .extend(entries.iter().cloned());
    }

    // 4. Walk binds and build bind index.
    for (name, declare) in &file_ast.defs {
        let def_id = DefId(*name);
        if file_ast.private_defs.contains(name) {
            typed.private_defs.insert(def_id);
        }
        let dependent_binder = BinderId::new(typed.file_id.0, BinderOwner::Definition(def_id.0));
        let own_tag_types: HashMap<Intern<String>, Ty> = typed
            .tag_types
            .iter()
            .map(|(id, ty)| (id.0, ty.clone()))
            .collect();
        let tag_types_by_name = resolved_types(&own_tag_types, &ctx.cross_file_tag_types);
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
                                Some((*name, Ty::i64()))
                            }
                            _ => None,
                        })
                        .collect::<HashMap<_, _>>(),
                )
            })
            .unwrap_or_default();
        let result_family_owner = file_ast.semantic_origin.as_ref().map_or_else(
            || {
                ast::ResultFamilyOwner::Callable(ast::ty::DeclarationKey::from_qualified_name(
                    ctx.package_instance.clone(),
                    *name,
                ))
            },
            |origin| {
                ast::ResultFamilyOwner::Callable(ast::ty::DeclarationKey::new(
                    origin.package.clone(),
                    origin.module.clone(),
                    declare.name,
                ))
            },
        );
        let mut method_tag_types = tag_types_by_name.clone();
        if let Some(receiver_type) = &receiver_type {
            method_tag_types.insert(Intern::<String>::from_ref("Self"), receiver_type.clone());
        }
        let return_ty = resolve_return_type(
            declare,
            &method_tag_types,
            &tag_params,
            &file_ast.tags,
            dependent_binder,
            &method_typevar_subst,
            receiver_type.as_ref(),
            declare
                .receiver_type_surface()
                .map(|receiver| &receiver.value),
            &result_family_owner,
        );
        let param_types = resolve_param_types(
            declare,
            &method_tag_types,
            &tag_params,
            &file_ast.tags,
            dependent_binder,
            &method_typevar_subst,
        );

        let param_kinds = declare
            .params
            .as_ref()
            .map(|params| {
                params
                    .iter()
                    .zip(&param_types)
                    .map(|((name, parameter), (_, ty))| {
                        if declare.param_refinements.contains_key(name) {
                            ParamKind::Value(Box::new(ty.clone()))
                        } else {
                            param_kind_from_parameter_kind(&parameter.kind, &tag_types_by_name)
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
        let param_refinements = declare
            .params
            .as_ref()
            .map(|params| {
                params
                    .keys()
                    .map(|name| declare.param_refinements.get(name).cloned())
                    .collect()
            })
            .unwrap_or_default();
        let (param_groups, groups, return_group) = resolve_target_groups(
            declare,
            &param_types,
            &param_conventions,
            &typed.type_registry,
            &mut typed.declaration_flaws,
        );
        let param_count = param_types.len();

        let intrinsic = declare
            .attributes
            .intrinsic
            .and_then(|name| crate::typed::IntrinsicOp::resolve(name.as_str()).ok());
        let mut bind_flaws = Vec::new();
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
        if let Some(role) = declare.attributes.invalid_operator_role {
            bind_flaws.push(
                Diagnostic::new(
                    "type-unknown-operator-role",
                    format!("unknown operator role `{}`", role.as_str()),
                )
                .with_arg("role", role.as_str().to_string())
                .at_span_id(
                    declare
                        .attributes
                        .operator_role_span
                        .unwrap_or(declare.name_span),
                    &typed.span_table,
                ),
            );
        }
        if declare.attributes.operator_role.is_some() && param_types.len() != 2 {
            bind_flaws.push(
                Diagnostic::new(
                    "type-invalid-operator-declaration",
                    "an operator implementation must have exactly two value parameters",
                )
                .at_span_id(declare.name_span, &typed.span_table),
            );
        }
        if matches!(
            declare.attributes.operator_role,
            Some(
                OperatorRole::Add
                    | OperatorRole::Subtract
                    | OperatorRole::Multiply
                    | OperatorRole::BitAnd
                    | OperatorRole::BitOr
                    | OperatorRole::BitXor
                    | OperatorRole::ShiftLeft
                    | OperatorRole::ShiftRight
            )
        ) && param_types
            .first()
            .is_some_and(|(_, operand)| operand != &return_ty)
        {
            bind_flaws.push(
                Diagnostic::new(
                    "type-invalid-operator-result",
                    "an integer operator implementation must return its left operand type",
                )
                .at_span_id(declare.name_span, &typed.span_table),
            );
        }
        if let Some(name) = declare.attributes.intrinsic
            && let Err(error) = crate::typed::IntrinsicOp::resolve(name.as_str())
        {
            bind_flaws.push(
                error
                    .diagnostic()
                    .at_span_id(declare.name_span, &typed.span_table),
            );
        }
        if let Some(intrinsic) = intrinsic
            && let Err(error) = intrinsic.validate_signature(
                &param_types
                    .iter()
                    .map(|(_, ty)| ty.clone())
                    .collect::<Vec<_>>(),
                &return_ty,
                Some(&typed.type_registry),
            )
        {
            bind_flaws.push(
                intrinsic
                    .diagnostic(&error)
                    .at_span_id(declare.name_span, &typed.span_table),
            );
        }
        let typed_bind = TypedBind {
            name: *name,
            dependent_binder,
            name_span: declare.name_span,
            return_type: return_ty.clone(),
            declared_return_type: return_ty.clone(),
            return_evidence: None,
            params: param_types,
            param_kinds,
            param_conventions,
            param_groups,
            param_refinements,
            groups,
            return_group,
            effects: Default::default(),
            receiver_type,
            unassigned_decl: matches!(declare.value, BindValue::Unassigned),
            is_extern: matches!(declare.value, BindValue::Extern),
            is_constant: declare.is_constant(),
            body: crate::typed::BindBody::Extern,
            flaws: bind_flaws,
            param_requirements: vec![AvailabilityRequirement::Any; param_count],
            call_capability: CallCapability::StagePolymorphic,
            signature_surface: declare.signature_surface(),
            doc_comment: declare.doc_comment.clone(),
            attributes: declare.attributes.clone(),
            intrinsic,
            operator_role: declare.attributes.operator_role,
        };
        if declare.params.is_some() {
            typed.fn_return_types.insert(def_id, return_ty.clone());
        }
        typed.defs.insert(def_id, typed_bind);
    }

    // 5. Build type scope for declarations.
    let scope_names = build_type_scope(file_ast, ctx);
    let type_scope: HashSet<Intern<String>> = scope_names.into_iter().collect();

    // 6. Check type annotations in declarations.
    let reflection_contract_issues = trait_registry.reflection_contract_with_issues(&typed);
    for issue in reflection_contract_issues.issues {
        let span = match &issue {
            ReflectionContractIssue::MissingRole { contract_span, .. } => *contract_span,
            ReflectionContractIssue::DuplicateRole { declarations, .. } => declarations
                .first()
                .map(|(_, span)| *span)
                .unwrap_or(SpanId::INVALID),
            ReflectionContractIssue::DuplicateReflectionTrait { declarations } => declarations
                .first()
                .map(|(_, span)| *span)
                .unwrap_or(SpanId::INVALID),
            ReflectionContractIssue::MalformedRole {
                declaration_span, ..
            } => *declaration_span,
            ReflectionContractIssue::RolesWithoutReflectionTrait { declarations } => declarations
                .first()
                .map(|(_, span)| *span)
                .unwrap_or(SpanId::INVALID),
            ReflectionContractIssue::ReflectionContractIsInvalid => SpanId::INVALID,
        };
        typed
            .declaration_flaws
            .push((span, issue.diagnostic().at_span_id(span, &typed.span_table)));
    }

    for declare in file_ast.tags.values() {
        if let Some(role) = declare.attributes.invalid_default_literal {
            typed.declaration_flaws.push((
                declare.name_span,
                Diagnostic::new(
                    "type-unknown-literal-default-role",
                    format!("unknown literal default role `{}`", role.as_str()),
                )
                .with_arg("role", role.as_str().to_string())
                .at_span_id(
                    declare
                        .attributes
                        .default_literal_span
                        .unwrap_or(declare.name_span),
                    &typed.span_table,
                ),
            ));
        }
        if let Some(role) = declare.attributes.invalid_default_type_former {
            typed.declaration_flaws.push((
                declare.name_span,
                Diagnostic::new(
                    "type-invalid-default-pointer-former-role",
                    format!("unknown default type-former role `{}`", role.as_str()),
                )
                .with_arg("role", role.as_str().to_string())
                .at_span_id(
                    declare
                        .attributes
                        .default_type_former_span
                        .unwrap_or(declare.name_span),
                    &typed.span_table,
                ),
            ));
        }
        for (role, span) in &declare.attributes.invalid_reflection_roles {
            typed.declaration_flaws.push((
                *span,
                Diagnostic::new(
                    "type-unknown-reflection-role",
                    format!("unknown reflection role `{}`", role.as_str()),
                )
                .with_arg("role", role.as_str().to_string())
                .at_span_id(*span, &typed.span_table),
            ));
        }
        if declare.attributes.default_type_former == Some(ast::DefaultTypeFormerRole::RawPointer) {
            let tag = typed.tags.get(&TagId(declare.name));
            let parameter_name = declare
                .params
                .as_ref()
                .and_then(|params| params.keys().next());
            let valid = declare
                .params
                .as_ref()
                .is_some_and(|params| params.len() == 1)
                && tag.is_some_and(|tag| {
                    crate::representation::Repr::derive(
                        &tag.resolved_ty,
                        Some(&typed.type_registry),
                    )
                    .is_ok_and(|repr| matches!(repr, crate::representation::Repr::Address { .. }))
                        && parameter_name.is_some_and(|parameter_name| {
                            typed
                            .type_registry
                            .resolved_definition_for_type(&tag.resolved_ty)
                            .pointee_ty().is_some_and(
                            |pointee| matches!(pointee, Ty::Opaque(name) if name == parameter_name),
                        )
                        })
                });
            if !valid {
                typed.declaration_flaws.push((
                    declare.name_span,
                    Diagnostic::new(
                        "type-invalid-default-pointer-former",
                        "the default raw-pointer former must be a one-parameter address type",
                    )
                    .at_span_id(declare.name_span, &typed.span_table),
                ));
            }
        }
        if declare.attributes.default_type_former == Some(ast::DefaultTypeFormerRole::FixedArray) {
            let valid = typed
                .tags
                .get(&TagId(declare.name))
                .and_then(|tag| tag.resolved_ty.named_instance())
                .and_then(|instance| typed.type_registry.declaration(instance.declaration))
                .is_some_and(|declaration| declaration.fixed_array.is_some());
            if !valid {
                typed.declaration_flaws.push((
                    declare.name_span,
                    Diagnostic::new(
                        "type-invalid-default-array-former",
                        "the default fixed-array former must carry a valid fixed-array contract",
                    )
                    .at_span_id(declare.name_span, &typed.span_table),
                ));
            }
        }
        if declare.attributes.default_literal == Some(ast::ty::LiteralKind::Integer)
            && typed.tags.get(&TagId(declare.name)).is_none_or(|tag| {
                tag.resolved_ty.type_id().is_none()
                    || crate::representation::Repr::derive(
                        &tag.resolved_ty,
                        Some(&typed.type_registry),
                    )
                    .is_err()
                    || typed
                        .type_registry
                        .integer_validity_for_type(&tag.resolved_ty)
                        .is_none()
            })
        {
            typed.declaration_flaws.push((
                declare.name_span,
                Diagnostic::new(
                    "type-invalid-integer-literal-default",
                    "the integer literal default must be a named bits type with integer validity",
                )
                .at_span_id(declare.name_span, &typed.span_table),
            ));
        }
        for provided in &declare.provided_traits {
            if !type_scope.contains(&provided.trait_name) {
                let diagnostic = Diagnostic::new(
                    "type-unknown-symbol",
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
                        Diagnostic::new(
                            "type-unknown-phantom-parameter",
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
                        Diagnostic::new(
                            "type-unused-generic-parameter",
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
            bind.flaws.push(
                Diagnostic::new(
                    "type-unsaturated-application",
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
        if let Some(typed_bind) = typed.defs.get_mut(&DefId(*name)) {
            typed_bind.flaws.push(
                Diagnostic::new(
                    "type-unsaturated-application",
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
                Diagnostic::new(
                    "type-unsaturated-application",
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

    for (tag_id, tag) in &typed.tags {
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
                Diagnostic::new(
                    "type-unsaturated-application",
                    format!("generic type `{}` requires type arguments", name.as_str()),
                )
                .with_arg("type_name", name.as_str().to_string())
                .at_span_id(tag.name_span, &typed.span_table),
            ));
        }
    }

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
        | Ty::Address { pointee: elem, .. }
        | Ty::Ref { inner: elem, .. } => unsaturated_type_name(elem, registry),
        Ty::Tuple(fields) => fields
            .iter()
            .find_map(|field| unsaturated_type_name(field, registry)),
        Ty::ResultFamily { .. }
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
            .is_some_and(|parameters| !parameters.is_empty())
            .then_some(*name),
        Expr::TagCall(call) => tag_params
            .get(&call.name)
            .is_some_and(|parameters| parameters.len() != call.args.len())
            .then_some(call.name)
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
    match ty {
        Ty::Ref { .. } | Ty::Ptr { .. } | Ty::Address { .. } => false,
        Ty::Named { instance, .. } => {
            declarations.contains_key(&instance.declaration.name)
                && declaration_recursively_refers_to_name(
                    &instance.declaration.name,
                    declarations,
                    resolved_tags,
                    recursion_stack,
                )
        }
        Ty::Opaque(name) => {
            declarations.contains_key(name)
                && declaration_recursively_refers_to_name(
                    name,
                    declarations,
                    resolved_tags,
                    recursion_stack,
                )
        }
        Ty::Record {
            fields,
            resolved_params,
            ..
        } => {
            fields.iter().any(|(_, ty)| {
                declaration_ty_refers_to_name(ty, declarations, resolved_tags, recursion_stack)
            }) || resolved_params.as_ref().is_some_and(|params| {
                params.iter().any(|(_, arg)| match arg {
                    ast::TyArg::Type(ty) => declaration_ty_refers_to_name(
                        ty,
                        declarations,
                        resolved_tags,
                        recursion_stack,
                    ),
                    ast::TyArg::Const(_) => false,
                })
            })
        }
        Ty::Union {
            variants,
            resolved_params,
            ..
        } => {
            variants.iter().any(|variant| {
                variant.fields.iter().any(|(_, ty)| {
                    declaration_ty_refers_to_name(ty, declarations, resolved_tags, recursion_stack)
                }) || variant.result_ty.as_ref().is_some_and(|ty| {
                    declaration_ty_refers_to_name(ty, declarations, resolved_tags, recursion_stack)
                })
            }) || resolved_params.as_ref().is_some_and(|params| {
                params.iter().any(|(_, arg)| match arg {
                    ast::TyArg::Type(ty) => declaration_ty_refers_to_name(
                        ty,
                        declarations,
                        resolved_tags,
                        recursion_stack,
                    ),
                    ast::TyArg::Const(_) => false,
                })
            })
        }
        Ty::Array { elem, .. } => {
            declaration_ty_refers_to_name(elem, declarations, resolved_tags, recursion_stack)
        }
        Ty::Tuple(elements) => elements.iter().any(|ty| {
            declaration_ty_refers_to_name(ty, declarations, resolved_tags, recursion_stack)
        }),
        Ty::AnonymousInteger { .. }
        | Ty::ResultFamily { .. }
        | Ty::Float { .. }
        | Ty::Unit
        | Ty::Literal(_)
        | Ty::UnresolvedLiteral(_) => false,
    }
}

fn declaration_reference_semantics(
    ty: &Ty,
    type_registry: &crate::type_registry::TypeRegistry,
) -> Result<Option<ReferenceSemantics>, Box<Diagnostic>> {
    let Some(reference) = reference_semantics_for_type(ty, Some(type_registry)) else {
        return Ok(None);
    };

    if is_nested_reference_former(ty, Some(type_registry)) {
        return Err(Box::new(Diagnostic::new(
            "type-invalid-reference-former",
            "reference former declarations cannot be nested",
        )));
    }

    Ok(Some(reference))
}

fn declaration_representation_type_root(ty: &Ty) -> Ty {
    declaration_representation_type(ty)
}

fn declaration_has_symbolic_fixed_array_length(
    ty: &Ty,
    registry: Option<&crate::TypeRegistry>,
) -> bool {
    declaration_has_symbolic_fixed_array_length_with_seen(ty, registry, &mut Vec::new())
}

fn declaration_has_symbolic_fixed_array_length_with_seen(
    ty: &Ty,
    registry: Option<&crate::TypeRegistry>,
    stack: &mut Vec<ast::ty::NamedTypeInstance>,
) -> bool {
    match ty {
        Ty::Array { size, .. } => match size.normalize().normalize_to_value() {
            Some(ast::ConstValue::Int(value)) => ast::integer::to_u64(value).is_none(),
            Some(_) => true,
            None => true,
        },
        Ty::Named { instance, .. } => {
            let Some(declaration) =
                registry.and_then(|registry| registry.declaration_for_instance(instance))
            else {
                return false;
            };

            if stack
                .iter()
                .any(|prior| prior.declaration == instance.declaration)
            {
                return false;
            }
            stack.push(instance.clone());

            let definition = &declaration.definition;
            let symbolic = if let Some(former) = declaration.fixed_array.as_ref() {
                let element = instance.arguments.iter().find_map(|(parameter, argument)| {
                    (*parameter == former.element_parameter).then_some(argument)
                });
                let length = instance.arguments.iter().find_map(|(parameter, argument)| {
                    (*parameter == former.length_parameter).then_some(argument)
                });
                if let Some(ast::TyArg::Type(_)) = element {
                    if let Some(ast::TyArg::Const(expr)) = length {
                        expr.normalize()
                            .normalize_to_value()
                            .is_none_or(|value| match value {
                                ast::ConstValue::Int(value) => {
                                    ast::integer::to_u64(value).is_none()
                                }
                                _ => true,
                            })
                    } else {
                        true
                    }
                } else {
                    false
                }
            } else {
                declaration_has_symbolic_fixed_array_length_with_seen(definition, registry, stack)
            };
            let _ = stack.pop();
            symbolic
        }
        Ty::Ref { inner, .. } | Ty::Ptr { inner, .. } | Ty::Address { pointee: inner, .. } => {
            declaration_has_symbolic_fixed_array_length_with_seen(inner, registry, stack)
        }
        Ty::Record { fields, .. } => fields.iter().any(|(_, field)| {
            declaration_has_symbolic_fixed_array_length_with_seen(field, registry, stack)
        }),
        Ty::Tuple(fields) => fields.iter().any(|field| {
            declaration_has_symbolic_fixed_array_length_with_seen(field, registry, stack)
        }),
        Ty::Union { variants, .. } => variants.iter().any(|variant| {
            variant.fields.iter().any(|(_, field)| {
                declaration_has_symbolic_fixed_array_length_with_seen(field, registry, stack)
            })
        }),
        _ => false,
    }
}

fn declaration_representation_type(ty: &Ty) -> Ty {
    match ty {
        Ty::Named { .. } => ty.clone(),
        Ty::Ref { inner, mutable } => Ty::Ref {
            inner: Box::new(declaration_representation_type(inner)),
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
                        Box::new(declaration_representation_type(field_ty)),
                    )
                })
                .collect(),
            resolved_params: resolved_params.clone(),
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
                    name: variant.name,
                    fields: variant
                        .fields
                        .iter()
                        .map(|(field_name, field_ty)| {
                            (
                                *field_name,
                                Box::new(declaration_representation_type(field_ty)),
                            )
                        })
                        .collect(),
                    result_ty: variant
                        .result_ty
                        .as_ref()
                        .map(declaration_representation_type),
                })
                .collect(),
            literal_values: literal_values.clone(),
            resolved_params: resolved_params.clone(),
        },
        Ty::Tuple(fields) => {
            Ty::Tuple(fields.iter().map(declaration_representation_type).collect())
        }
        Ty::Array { elem, size } => Ty::Array {
            elem: Box::new(declaration_representation_type(elem)),
            size: size.clone(),
        },
        Ty::Ptr { inner } => Ty::Ptr {
            inner: Box::new(declaration_representation_type(inner)),
        },
        Ty::Address {
            pointee,
            address_space,
        } => Ty::Address {
            pointee: Box::new(declaration_representation_type(pointee)),
            address_space: *address_space,
        },
        Ty::Opaque(name) if name.as_str() == "Int" => Ty::i64(),
        ty => ty.clone(),
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
        .map(|(parameter, _)| {
            (
                *parameter,
                ast::TyArg::Type(Box::new(Ty::Opaque(*parameter))),
            )
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
            resolved_params: resolved_params.as_ref().map(|params| {
                params
                    .iter()
                    .map(|(name, arg)| {
                        let arg = match arg {
                            ast::TyArg::Type(ty) => {
                                ast::TyArg::Type(Box::new(preserve_self_reference_with_instance(
                                    ty,
                                    declaration_name,
                                    instance,
                                )))
                            }
                            ast::TyArg::Const(expr) => ast::TyArg::Const(expr.clone()),
                        };
                        (*name, arg)
                    })
                    .collect()
            }),
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
            resolved_params: resolved_params.as_ref().map(|params| {
                params
                    .iter()
                    .map(|(name, arg)| {
                        let arg = match arg {
                            ast::TyArg::Type(ty) => {
                                ast::TyArg::Type(Box::new(preserve_self_reference_with_instance(
                                    ty,
                                    declaration_name,
                                    instance,
                                )))
                            }
                            ast::TyArg::Const(expr) => ast::TyArg::Const(expr.clone()),
                        };
                        (*name, arg)
                    })
                    .collect()
            }),
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
        Ty::Tuple(elements) => Ty::Tuple(
            elements
                .iter()
                .map(|element| {
                    preserve_self_reference_with_instance(element, declaration_name, instance)
                })
                .collect(),
        ),
        Ty::Literal(v) => Ty::Literal(v.clone()),
        Ty::AnonymousInteger { .. }
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
    names.extend(file_ast.defs.keys().copied());
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
            ImportSource::LocalMember(m) => {
                names.insert(m.member.alias.unwrap_or(m.member.export));
            }
            ImportSource::Package(_) | ImportSource::Local(_, _) => {
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
                    Diagnostic::new(
                        "type-unknown-symbol",
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
                                Diagnostic::new(
                                    "type-unknown-symbol",
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
                                Diagnostic::new(
                                    "type-unknown-symbol",
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

fn check_qualified_has_members(
    declare: &ast::Declare,
    file_ast: &FileAst,
    eval_ast: &FileAst,
    push_flaw: &mut dyn FnMut(SpanId, Diagnostic),
) {
    let DeclareValue::Has(members) = &declare.value else {
        return;
    };

    for member in members {
        let qualifier = match member {
            ast::HasMember::Property(property) => property.qualifier.as_ref(),
            ast::HasMember::Function(function) => function.qualifier.as_ref(),
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
                Diagnostic::new(
                    "type-uninherited-trait-qualifier",
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

        let mut visited = HashSet::new();
        let ast::HasMember::Function(function) = member else {
            continue;
        };
        match trait_method_lookup(
            qualifier.name,
            function.name,
            file_ast,
            eval_ast,
            &mut visited,
        ) {
            TraitMethodLookup::Found => {}
            TraitMethodLookup::Missing => push_flaw(
                function.name_span,
                Diagnostic::new(
                    "type-unknown-trait-method",
                    format!(
                        "trait `{}` has no method `{}`",
                        qualifier.name.as_str(),
                        function.name.as_str()
                    ),
                )
                .with_arg("trait_name", qualifier.name.as_str().to_string())
                .with_arg("method_name", function.name.as_str().to_string()),
            ),
            TraitMethodLookup::UnknownTrait(name) => push_flaw(
                qualifier.span,
                Diagnostic::new(
                    "type-unknown-trait",
                    format!("unknown trait `{}`", name.as_str()),
                )
                .with_arg("trait_name", name.as_str().to_string()),
            ),
        }
    }
}

enum TraitMethodLookup {
    Found,
    Missing,
    UnknownTrait(Intern<String>),
}

fn trait_method_lookup(
    trait_name: Intern<String>,
    method_name: Intern<String>,
    file_ast: &FileAst,
    eval_ast: &FileAst,
    visited: &mut HashSet<Intern<String>>,
) -> TraitMethodLookup {
    if !visited.insert(trait_name) {
        return TraitMethodLookup::Missing;
    }
    let Some(declaration) = file_ast
        .tags
        .get(&trait_name)
        .or_else(|| eval_ast.tags.get(&trait_name))
    else {
        return TraitMethodLookup::UnknownTrait(trait_name);
    };
    if let DeclareValue::Has(members) = &declaration.value
        && members.iter().any(|member| {
            matches!(member, ast::HasMember::Function(function) if function.name == method_name)
        })
    {
        return TraitMethodLookup::Found;
    }
    let mut unknown_trait = None;
    for provided in &declaration.provided_traits {
        match trait_method_lookup(
            provided.trait_name,
            method_name,
            file_ast,
            eval_ast,
            visited,
        ) {
            TraitMethodLookup::Found => return TraitMethodLookup::Found,
            TraitMethodLookup::Missing => {}
            TraitMethodLookup::UnknownTrait(name) => unknown_trait = Some(name),
        }
    }
    if let Some(name) = unknown_trait {
        TraitMethodLookup::UnknownTrait(name)
    } else {
        TraitMethodLookup::Missing
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
                Diagnostic::new(
                    "no-self-in-associated-method",
                    "associated method has no `self` parameter",
                )
                .with_help("add a `self`, `ref self`, or `mut self` parameter, or call through an explicit value"),
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
            if let Some(expr) = &ret.value {
                collect_self_ref_spans(expr, spans);
            }
        }
        BindValue::Extern | BindValue::Unassigned => {}
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
                Diagnostic::new(
                    "type-in-range-on-bounded-int-tag",
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
    if let Some(reference) = reference_semantics_for_type(ty, None) {
        return references_nominal_type(&reference.pointee, target);
    }

    match ty {
        Ty::Ref { inner, .. } => references_nominal_type(inner, target),
        Ty::Named { instance, name } => {
            *name == target
                || instance
                    .arguments
                    .iter()
                    .any(|(_, argument)| match argument {
                        ast::TyArg::Type(ty) => references_nominal_type(ty, target),
                        ast::TyArg::Const(_) => false,
                    })
        }
        Ty::Record {
            name,
            fields,
            resolved_params,
        } => {
            *name == target
                || fields
                    .iter()
                    .any(|(_, field_ty)| references_nominal_type(field_ty, target))
                || resolved_params.as_ref().is_some_and(|params| {
                    params.iter().any(|(_, arg)| match arg {
                        ast::TyArg::Type(ty) => references_nominal_type(ty, target),
                        ast::TyArg::Const(_) => false,
                    })
                })
        }
        Ty::Union {
            name,
            variants,
            resolved_params,
            ..
        } => {
            *name == target
                || variants.iter().any(|variant| {
                    variant
                        .fields
                        .iter()
                        .any(|(_, field_ty)| references_nominal_type(field_ty, target))
                        || variant
                            .result_ty
                            .as_ref()
                            .is_some_and(|ty| references_nominal_type(ty, target))
                })
                || resolved_params.as_ref().is_some_and(|params| {
                    params.iter().any(|(_, arg)| match arg {
                        ast::TyArg::Type(ty) => references_nominal_type(ty, target),
                        ast::TyArg::Const(_) => false,
                    })
                })
        }
        Ty::Opaque(name) => *name == target,
        Ty::Array { elem, .. } | Ty::Ptr { inner: elem } | Ty::Address { pointee: elem, .. } => {
            references_nominal_type(elem, target)
        }
        Ty::Tuple(elements) => elements
            .iter()
            .any(|element| references_nominal_type(element, target)),
        Ty::AnonymousInteger { .. }
        | Ty::ResultFamily { .. }
        | Ty::Float { .. }
        | Ty::Unit
        | Ty::Literal(_)
        | Ty::UnresolvedLiteral(_) => false,
    }
}

fn resolve_tag_pass(
    tag_name: &Intern<String>,
    declare: &ast::declare::Declare,
    all_tags: &ast::TagMap,
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
            let is_transparent_alias = matches!(&declare.value, DeclareValue::Alias(alias)
                if !matches!(&alias.value, Expr::TakePtr(_))
            );
            if is_transparent_alias {
                return (*name, definition.clone());
            }
            let instance = ast::ty::NamedTypeInstance::new(ast::ty::TypeId {
                file: file_id.0,
                name: *name,
            })
            .with_arguments(declaration_instance_arguments(declare));
            (
                *name,
                Ty::Named {
                    instance,
                    name: *name,
                },
            )
        })
        .collect()
}

pub(crate) fn resolve_tags_fixpoint(
    tags: &ast::TagMap,
    tag_params: &HashMap<Intern<String>, Parameters>,
    cross_file_tag_types: &HashMap<TagId, Ty>,
    file_id: FileId,
    max_passes: usize,
) -> HashMap<Intern<String>, Ty> {
    let mut resolved: HashMap<Intern<String>, Ty> = HashMap::new();
    for _pass in 0..max_passes {
        let available = nominal_types_for_definitions(&resolved, tags, file_id);
        let next: HashMap<_, _> = tags
            .iter()
            .map(|(name, declare)| {
                (
                    *name,
                    resolve_tag_pass(
                        name,
                        declare,
                        tags,
                        tag_params,
                        cross_file_tag_types,
                        &available,
                        file_id,
                    ),
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
        Pattern::Literal(Literal::String(s), _) => Some(ConstValue::String(s.clone())),
        Pattern::Literal(Literal::Int(n), _) => Some(ConstValue::Int(*n)),
        Pattern::Literal(Literal::Number(n), _) => Some(ConstValue::Int((*n as u128).into())),
        Pattern::Literal(Literal::Float(HashFloat(f)), _) => Some(ConstValue::Float(HashFloat(*f))),
        _ => None,
    }
}

fn resolve_declare_value(
    declaring_name: &Intern<String>,
    value: &DeclareValue,
    all_tags: &ast::TagMap,
    tag_params: &HashMap<Intern<String>, Parameters>,
    cross_file_tag_types: &HashMap<TagId, Ty>,
    resolved: &HashMap<Intern<String>, Ty>,
    _file_id: FileId,
) -> Ty {
    let tag_types = resolved_types(resolved, cross_file_tag_types);
    let env = TypeEnv::new(&tag_types)
        .with_tag_params(tag_params)
        .with_tag_decls(all_tags);

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
                                        ParameterKind::Generic => Ty::Opaque(*name),
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
                            (*name, Vec::<(Intern<String>, Box<Ty>)>::new())
                        }
                        _ => (
                            Intern::new(String::new()),
                            Vec::<(Intern<String>, Box<Ty>)>::new(),
                        ),
                    };
                    let result_ty = v.result_ty().map(|rt| env.resolve_expr(&rt.value));
                    UnionVariant {
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
        Expr::TagCall(tc) => tag_types
            .get(&tc.name)
            .cloned()
            .unwrap_or(Ty::Opaque(tc.name)),
        _ => Ty::Unit,
    }
}

fn resolve_target_groups(
    bind: &Bind,
    param_types: &[(Intern<String>, Ty)],
    param_conventions: &[ParamConvention],
    type_registry: &crate::type_registry::TypeRegistry,
    flaws: &mut Vec<(SpanId, Diagnostic)>,
) -> (Vec<Option<GroupId>>, Vec<TypedGroup>, Option<GroupId>) {
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
                        Diagnostic::new(
                            "type-incompatible-target-group",
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
                Diagnostic::new(
                    "type-unknown-target-group",
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
                        Diagnostic::new(
                            "type-incompatible-target-group",
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
                        Diagnostic::new(
                            "type-unknown-target-group",
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
                        Diagnostic::new(
                            "type-ambiguous-reference-return-target-group",
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

    (param_groups, groups, return_group)
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
            Self::UnknownField(field) => Diagnostic::new(
                "type-unknown-target-group-field",
                format!("target group `{path}` refers to unknown field `{field}`"),
            )
            .with_arg("group", path.to_string())
            .with_arg("field", field.to_string()),
            Self::UnknownVariant(variant) => Diagnostic::new(
                "type-unknown-target-group-variant",
                format!("target group `{path}` refers to unknown variant `{variant}`"),
            )
            .with_arg("group", path.to_string())
            .with_arg("variant", variant.to_string()),
            Self::InvalidProjection { segment, ty } => Diagnostic::new(
                "type-invalid-target-group-projection",
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
            Self::Variant(fields) => Ty::Tuple(fields.into_iter().map(|(_, ty)| *ty).collect()),
        }
    }

    fn unwrapped(self, type_registry: &crate::type_registry::TypeRegistry) -> Self {
        match self {
            Self::Type(ty) => {
                if let Some(reference) = reference_semantics_for_type(&ty, Some(type_registry)) {
                    Self::Type(reference.pointee).unwrapped(type_registry)
                } else {
                    match ty {
                        Ty::Named { .. } => {
                            Self::Type(type_registry.resolved_definition_for_type(&ty))
                                .unwrapped(type_registry)
                        }
                        _ => Self::Type(ty),
                    }
                }
            }
            cursor => cursor,
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
        match segment.as_str() {
            "items" => match cursor {
                GroupPathCursor::Type(Ty::Array { elem, .. }) => {
                    projections.push(GroupProjection::Items);
                    cursor = GroupPathCursor::Type(*elem);
                }
                cursor => {
                    return Err(GroupPathError::InvalidProjection {
                        segment: *segment,
                        ty: Box::new(cursor.into_type()),
                    });
                }
            },
            "pointee" => match cursor {
                GroupPathCursor::Type(ty) if ty.pointee_ty().is_some() => {
                    projections.push(GroupProjection::Pointee);
                    cursor = GroupPathCursor::Type(ty.pointee_ty().cloned().unwrap());
                }
                cursor => {
                    return Err(GroupPathError::InvalidProjection {
                        segment: *segment,
                        ty: Box::new(cursor.into_type()),
                    });
                }
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
                    cursor => {
                        return Err(GroupPathError::InvalidProjection {
                            segment: *segment,
                            ty: Box::new(cursor.into_type()),
                        });
                    }
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
                cursor => {
                    return Err(GroupPathError::InvalidProjection {
                        segment: *segment,
                        ty: Box::new(cursor.into_type()),
                    });
                }
            },
        }
    }
    Ok((projections, cursor.unwrapped(type_registry).into_type()))
}

#[allow(clippy::too_many_arguments)]
fn resolve_return_type(
    bind: &Bind,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: &ast::TagMap,
    dependent_binder: BinderId,
    typevar_subst: &HashMap<Intern<String>, Ty>,
    receiver_type: Option<&Ty>,
    receiver_type_surface: Option<&ast::Expr>,
    result_family_owner: &ast::ResultFamilyOwner,
) -> Ty {
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
                    proposition: None,
                })
                .collect(),
        };
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
            .with_subst(typevar_subst)
            .with_tag_params(tag_params)
            .with_tag_decls(tag_decls)
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
            .with_subst(typevar_subst)
            .with_tag_params(tag_params)
            .with_tag_decls(tag_decls)
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
                    ParameterKind::Generic => Ty::Opaque(parameter),
                    ParameterKind::Tagged(surface)
                    | ParameterKind::ValueParam { ty: surface }
                    | ParameterKind::Inferred { ty: surface } => TypeEnv::new(tag_types)
                        .with_subst(typevar_subst)
                        .with_tag_params(tag_params)
                        .with_tag_decls(tag_decls)
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
        BindValue::Body { ret, .. } => ret
            .value
            .as_ref()
            .map(|e| resolve_type_from_typed_expr(e, tag_types))
            .unwrap_or(Ty::Unit),
        _ => Ty::Unit,
    }
}

fn inferred_result_alternatives(value: &BindValue, tag_decls: &ast::TagMap) -> Vec<Intern<String>> {
    let mut alternatives = Vec::new();
    match value {
        BindValue::Expr(expr) => {
            collect_result_exit_alternatives(&expr.value, tag_decls, &mut alternatives)
        }
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                if matches!(expr.value, Expr::If(_) | Expr::When(_)) {
                    collect_result_exit_alternatives(&expr.value, tag_decls, &mut alternatives);
                }
            }
            if let Some(ret) = &ret.value {
                collect_result_exit_alternatives(&ret.value, tag_decls, &mut alternatives);
            }
        }
        BindValue::Extern | BindValue::Unassigned => {}
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
                .filter_map(|(name, bind)| bind.is_constant().then_some(*name)),
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
                Diagnostic::new(
                    "type-result-proof-conflict",
                    format!(
                        "alternative `{}` has conflicting `thus` contracts",
                        alternative.value.label
                    ),
                )
                .with_arg("alternative", alternative.value.label.as_str().to_string())
                .at_span_id(alternative.span_id, span_table),
            );
        }
        if matches!(bind.value, BindValue::Extern) {
            flaws.push(
                Diagnostic::new(
                    "type-extern-proof-contract-untrusted",
                    "an extern callable cannot assert an unverified proof contract",
                )
                .at_span_id(alternative.span_id, span_table),
            );
        }
        let mut names = HashSet::new();
        proof_proposition_names(proposition, &mut names);
        for name in names.difference(&allowed_names) {
            flaws.push(
                Diagnostic::new(
                    "type-proof-contract-unknown-name",
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
    if contracts.is_empty() || matches!(bind.value, BindValue::Extern | BindValue::Unassigned) {
        return;
    }
    let mut facts = Vec::new();
    match &bind.value {
        BindValue::Expr(expr) => verify_result_exit(expr, &facts, &contracts, flaws, span_table),
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                if let Expr::If(if_expr) = &expr.value {
                    let mut taken = facts.clone();
                    taken.extend(proof_facts_for_condition(&if_expr.condition, true));
                    if let Some(exit) = if_expr.body.last() {
                        verify_result_exit(exit, &taken, &contracts, flaws, span_table);
                    }
                    facts.extend(proof_facts_for_condition(&if_expr.condition, false));
                }
            }
            if let Some(exit) = &ret.value {
                verify_result_exit(exit, &facts, &contracts, flaws, span_table);
            }
        }
        BindValue::Extern | BindValue::Unassigned => {}
    }
}

fn verify_result_exit(
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
            "type-result-proof-failed",
            format!("construction of `{label}` contradicts its `thus` contract `{contract}`"),
        ))
    } else {
        Some((
            "type-result-proof-unproven",
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
    if facts.contains(proposition) {
        return true;
    }
    match proposition {
        ast::ProofProposition::Compare {
            left,
            relation: ast::ProofRelation::Equal,
            right,
        } => left == right,
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

fn proof_facts_for_condition(
    condition: &ast::Condition,
    taken: bool,
) -> Vec<ast::ProofProposition> {
    match condition {
        ast::Condition::Is { subject, pattern } => {
            let ast::Pattern::Nominal(label, _) = &pattern.value else {
                return Vec::new();
            };
            let Some(proposition) = proof_for_comparison_expr(&subject.value) else {
                return Vec::new();
            };
            let selected_true = (label.as_str() == "True") == taken;
            if selected_true {
                vec![proposition]
            } else {
                proof_complement(&proposition).into_iter().collect()
            }
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
    let relation = match binary.op {
        ast::BinOp::Equal => ast::ProofRelation::Equal,
        ast::BinOp::Less => ast::ProofRelation::Less,
        ast::BinOp::LessOrEqual => ast::ProofRelation::LessOrEqual,
        ast::BinOp::Greater => ast::ProofRelation::Greater,
        ast::BinOp::GreaterOrEqual => ast::ProofRelation::GreaterOrEqual,
        _ => return None,
    };
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
        Expr::Lit(Literal::Number(value)) => Some(ast::ProofTerm::Value((*value as u128).into())),
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
    let relation = match relation {
        ast::ProofRelation::Equal => ast::ProofRelation::NotEqual,
        ast::ProofRelation::NotEqual => ast::ProofRelation::Equal,
        ast::ProofRelation::Less => ast::ProofRelation::GreaterOrEqual,
        ast::ProofRelation::LessOrEqual => ast::ProofRelation::Greater,
        ast::ProofRelation::Greater => ast::ProofRelation::LessOrEqual,
        ast::ProofRelation::GreaterOrEqual => ast::ProofRelation::Less,
    };
    Some(ast::ProofProposition::Compare {
        left: left.clone(),
        relation,
        right: right.clone(),
    })
}

fn proof_proposition_names(
    proposition: &ast::ProofProposition,
    names: &mut HashSet<Intern<String>>,
) {
    match proposition {
        ast::ProofProposition::Compare { left, right, .. } => {
            proof_term_names(left, names);
            proof_term_names(right, names);
        }
        ast::ProofProposition::InRange { value, start, end } => {
            proof_term_names(value, names);
            proof_term_names(start, names);
            proof_term_names(end, names);
        }
        ast::ProofProposition::Not(inner) => proof_proposition_names(inner, names),
        ast::ProofProposition::And(left, right) | ast::ProofProposition::Or(left, right) => {
            proof_proposition_names(left, names);
            proof_proposition_names(right, names);
        }
    }
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
    predicate_expr_names(predicate, &mut names);
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
                    && call.path.value.segments.is_empty()
                    && target_dependent_names.contains(&call.path.value.root)
        )
}

fn predicate_expr_names(predicate: &ast::ty::PredicateExpr, names: &mut HashSet<Intern<String>>) {
    match predicate {
        ast::ty::PredicateExpr::Lt(expr)
        | ast::ty::PredicateExpr::Gt(expr)
        | ast::ty::PredicateExpr::Le(expr)
        | ast::ty::PredicateExpr::Ge(expr)
        | ast::ty::PredicateExpr::Eq(expr)
        | ast::ty::PredicateExpr::Ne(expr) => normal_expr_names(expr, names),
        ast::ty::PredicateExpr::And(predicates) => {
            for predicate in predicates {
                predicate_expr_names(predicate, names);
            }
        }
        ast::ty::PredicateExpr::Proposition(proposition) => {
            proof_proposition_names(proposition, names);
        }
    }
}

fn normal_expr_names(expr: &ast::NormalExpr, names: &mut HashSet<Intern<String>>) {
    match expr {
        ast::NormalExpr::Var(name) => {
            names.insert(*name);
        }
        ast::NormalExpr::Add(left, right)
        | ast::NormalExpr::Sub(left, right)
        | ast::NormalExpr::Mul(left, right) => {
            normal_expr_names(left, names);
            normal_expr_names(right, names);
        }
        ast::NormalExpr::TargetQuery { operand, .. } => type_reference_names(operand, names),
        ast::NormalExpr::Value(_) | ast::NormalExpr::Inferred(_) => {}
    }
}

fn proof_term_names(term: &ast::ProofTerm, names: &mut HashSet<Intern<String>>) {
    match term {
        ast::ProofTerm::Name(name) => {
            names.insert(*name);
        }
        ast::ProofTerm::Add(left, right)
        | ast::ProofTerm::Sub(left, right)
        | ast::ProofTerm::Mul(left, right)
        | ast::ProofTerm::Remainder(left, right) => {
            proof_term_names(left, names);
            proof_term_names(right, names);
        }
        ast::ProofTerm::PowerOfTwo(inner) => proof_term_names(inner, names),
        ast::ProofTerm::TargetQuery { operand, .. } => type_reference_names(operand, names),
        ast::ProofTerm::Value(_) => {}
    }
}

fn type_reference_names(reference: &ast::TypeReference, names: &mut HashSet<Intern<String>>) {
    match reference {
        ast::TypeReference::Unit => {}
        ast::TypeReference::Nominal(name) => {
            names.insert(*name);
        }
        ast::TypeReference::Application { name, arguments } => {
            names.insert(*name);
            for argument in arguments {
                type_reference_names(argument, names);
            }
        }
        ast::TypeReference::Pointer(inner) => type_reference_names(inner, names),
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
                .chain(inferred_result_alternatives(&bind.value, &file_ast.tags))
        })
        .collect()
}

fn contains_unresolved_result_label(ty: &Ty, labels: &HashSet<Intern<String>>) -> bool {
    match ty {
        Ty::Opaque(name) => labels.contains(name),
        Ty::Named { instance, .. } => {
            instance
                .arguments
                .iter()
                .any(|(_, argument)| match argument {
                    ast::TyArg::Type(ty) => contains_unresolved_result_label(ty, labels),
                    ast::TyArg::Const(_) => false,
                })
        }
        Ty::Array {
            elem: underlying, ..
        }
        | Ty::Ptr { inner: underlying }
        | Ty::Address {
            pointee: underlying,
            ..
        }
        | Ty::Ref {
            inner: underlying, ..
        } => contains_unresolved_result_label(underlying, labels),
        Ty::Record { fields, .. } => fields
            .iter()
            .any(|(_, field)| contains_unresolved_result_label(field, labels)),
        Ty::Union { variants, .. } => variants.iter().any(|variant| {
            variant
                .fields
                .iter()
                .any(|(_, field)| contains_unresolved_result_label(field, labels))
                || variant
                    .result_ty
                    .as_ref()
                    .is_some_and(|result| contains_unresolved_result_label(result, labels))
        }),
        _ => false,
    }
}

fn anonymous_result_annotation_diagnostic(
    span: ast::SpanId,
    span_table: &ast::SpanTable,
) -> Diagnostic {
    Diagnostic::new(
        "type-anonymous-result-not-nameable",
        "an anonymous result-family type cannot be named in a source annotation",
    )
    .with_help("use inference, transparent forwarding, or translate to a named union")
    .at_span_id(span, span_table)
}

fn collect_result_exit_alternatives(
    expr: &Expr,
    tag_decls: &ast::TagMap,
    alternatives: &mut Vec<Intern<String>>,
) {
    let label = match expr {
        Expr::AnonymousTag(label) => Some(*label),
        Expr::TagCall(call) if call.qual_path.is_none() && !tag_decls.contains_key(&call.name) => {
            Some(call.name)
        }
        _ => None,
    };
    if let Some(label) = label {
        if !alternatives.contains(&label) {
            alternatives.push(label);
        }
        return;
    }
    match expr {
        Expr::If(if_expr) => {
            if let Some(exit) = if_expr.body.last() {
                collect_result_exit_alternatives(&exit.value, tag_decls, alternatives);
            }
            if let Some(exit) = &if_expr.ret.value {
                collect_result_exit_alternatives(&exit.value, tag_decls, alternatives);
            }
        }
        Expr::When(when_expr) => {
            for arm in &when_expr.arms {
                let body = match arm {
                    ast::WhenArm::Cond { body, .. }
                    | ast::WhenArm::Is { body, .. }
                    | ast::WhenArm::Else(body, _) => body,
                };
                collect_result_exit_alternatives(&body.value, tag_decls, alternatives);
            }
        }
        _ => {}
    }
}

fn resolve_param_types(
    bind: &Bind,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: &ast::TagMap,
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
                    .with_subst(typevar_subst)
                    .with_tag_params(tag_params)
                    .with_tag_decls(tag_decls)
                    .with_dependent_binder(dependent_binder)
                    .resolve_expr(&sp.value),
                ParameterKind::ValueParam { ty } | ParameterKind::Inferred { ty } => {
                    TypeEnv::new(tag_types)
                        .with_subst(typevar_subst)
                        .with_tag_params(tag_params)
                        .with_tag_decls(tag_decls)
                        .with_dependent_binder(dependent_binder)
                        .resolve_expr(&ty.value)
                }
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
            (*name, ty)
        })
        .collect()
}

fn validated_fixed_array_former(declare: &ast::Declare) -> Option<crate::FixedArrayFormer> {
    let contract = declare.attributes.fixed_array.as_ref()?;
    let params = declare.params.as_ref()?;
    if contract.element_parameter == contract.length_parameter {
        return None;
    }
    let element_is_type = params
        .get(&contract.element_parameter)
        .is_some_and(|parameter| match &parameter.kind {
            ParameterKind::Generic => true,
            ParameterKind::Tagged(ty) => is_type_param_marker(&ty.value),
            ParameterKind::ValueParam { ty } | ParameterKind::Inferred { ty } => {
                is_type_param_marker(&ty.value)
            }
            _ => false,
        });
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
    (element_is_type && length_is_const).then_some(crate::FixedArrayFormer {
        element_parameter: contract.element_parameter,
        length_parameter: contract.length_parameter,
    })
}

fn is_type_param_marker(ty: &Expr) -> bool {
    matches!(ty, Expr::AnonymousTag(name) if name.as_str() == "Type")
}

fn param_kind_from_parameter_kind(
    kind: &ParameterKind,
    tag_types: &HashMap<Intern<String>, Ty>,
) -> ParamKind {
    match kind {
        ParameterKind::Generic => ParamKind::Type,
        ParameterKind::Tagged(sp) if is_type_param_marker(&sp.value) => ParamKind::Type,
        ParameterKind::Tagged(sp) => {
            let ty = TypeEnv::new(tag_types).resolve_expr(&sp.value);
            ParamKind::Value(Box::new(ty))
        }
        ParameterKind::ValueParam { ty } | ParameterKind::Inferred { ty } => {
            let ty = TypeEnv::new(tag_types).resolve_expr(&ty.value);
            ParamKind::Value(Box::new(ty))
        }
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

#[cfg(test)]
#[path = "../../tests/transform_declare_tests.rs"]
mod tests;
