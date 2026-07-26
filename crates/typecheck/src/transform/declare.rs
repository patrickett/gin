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
use crate::analysis::TypeEnv;
use crate::compile_time_trait::{RESERVED_TRAITS, synthesize_reflectable_trait};
use crate::ty::{Ty, UnionVariant};
use crate::typed::{
    DefId, ExprId, FileId, GroupId, GroupProjection, TagId, TypedBind, TypedFileAst, TypedGroup,
    TypedTag, VariantMap,
};
use ast::parameter::Parameters;
use ast::prelude::*;
use ast::type_decl::TypeNameExt;
use ast::{
    BindValue, BinderId, BinderOwner, ConstValue, DeclareValue, HashFloat, ImportSource, Literal,
    ParamKind, TypeExpr,
};

use ast_format::declare::{BindFormatExt, DeclareFormatExt};
use ast_format::type_expr::TypeExprFormatExt;

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
            .filter_map(|(name, kind)| {
                let kind = match kind {
                    ParameterKind::Generic => ParamKind::Type,
                    ParameterKind::Tagged(sp) if is_type_param_marker(&sp.value) => ParamKind::Type,
                    ParameterKind::Tagged(sp) => {
                        let ty = TypeEnv::new(tag_types).resolve(&sp.value);
                        ParamKind::Value(Box::new(ty))
                    }
                    ParameterKind::ValueParam { ty } | ParameterKind::Inferred { ty } => {
                        let ty = TypeEnv::new(tag_types).resolve(&ty.value);
                        ParamKind::Value(Box::new(ty))
                    }
                    ParameterKind::Default(expr) => {
                        let te = expr.value.as_type_expr()?;
                        let ty = TypeEnv::new(tag_types).resolve(&te);
                        ParamKind::Value(Box::new(ty))
                    }
                };
                Some((*name, kind))
            })
            .collect()
    }
}

/// Takes `&FileAst` and returns a `TypedFileAst` with declarations
/// resolved and the file-level index populated. The expression arena is left
/// empty — it will be filled in Stage 2.
pub fn stage_declare(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    let mut typed = TypedFileAst::new(file_id, file_ast.span_table.clone());
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
        TAG_RESOLVE_MAX_PASSES,
    );
    for (name, declare) in &file_ast.tags {
        let tag_id = TagId(*name);
        let resolved_ty = resolved_tags.get(name).cloned().unwrap_or_else(|| {
            resolve_declare_value(
                name,
                &declare.value,
                &file_ast.tags,
                &file_ast.tags,
                &tag_params,
                &ctx.cross_file_tag_types,
                &resolved_tags,
            )
        });

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
            if RESERVED_TRAITS.contains(&pt.trait_name.as_str()) {
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
        provided_traits.retain(|pt| !RESERVED_TRAITS.contains(&pt.trait_name.as_str()));

        // Register provision field expression spans in the typed expression index
        // so `self` in `Type.Trait has field: self.member` can be found by hover/go-to-def.
        let record_fields = if let Ty::Record { ref fields, .. } = resolved_ty {
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
                    typed.exprs.target_group.push(None);
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
        if !ctx.ide_package && name.as_str() != crate::compile_time_trait::REFLECTABLE_TRAIT {
            provided_traits.push(synthesize_reflectable_trait(&resolved_ty));
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
                            // Property: `name Type`
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
                                for (param_name, kind) in &f.params {
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
                                    match kind {
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
    rebuild_own_variant_map(&mut typed.variant_map, &typed.tag_types, &own_unions);

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
                    TypeExpr::Nominal(name, _) => name.as_str(),
                    TypeExpr::Generic { name, .. } => name.as_str(),
                    TypeExpr::Qualified(path) => path
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

    // 4. Walk public binds and build bind index.
    for (name, declare) in &file_ast.defs {
        if file_ast.private_defs.contains(name) {
            continue;
        }
        let def_id = DefId(*name);
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
                .resolve(&receiver.value)
        });
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
        );
        let param_types = resolve_param_types(
            declare,
            &method_tag_types,
            &tag_params,
            &file_ast.tags,
            dependent_binder,
        );

        let param_kinds = declare
            .params
            .as_ref()
            .map(|params| {
                params
                    .iter()
                    .map(|(_, k)| param_kind_from_parameter_kind(k, &tag_types_by_name))
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
            &mut typed.declaration_flaws,
        );

        let typed_bind = TypedBind {
            name: *name,
            dependent_binder,
            name_span: declare.name_span,
            is_compile_time: declare.is_compile_time,
            return_type: return_ty.clone(),
            declared_return_type: return_ty.clone(),
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
            is_constant: declare.is_constant,
            body: crate::typed::BindBody::Extern,
            flaws: Vec::new(),
            signature_surface: declare.signature_surface(),
            doc_comment: declare.doc_comment.clone(),
            attributes: declare.attributes.clone(),
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
    for declare in file_ast.tags.values() {
        check_declare_value_type_scope(&declare.value, &type_scope, &mut |span, symptom| {
            let symptom = symptom.at_span_id(span, &typed.span_table);
            typed.declaration_flaws.push((span, symptom))
        });
    }

    typed
}

fn build_type_scope(file_ast: &FileAst, ctx: &TransformCtx) -> HashSet<Intern<String>> {
    let mut names: HashSet<Intern<String>> = HashSet::new();
    names.extend(file_ast.tags.keys().copied());
    names.extend(file_ast.defs.keys().copied());
    names.extend(ctx.cross_file_tag_types.keys().map(|t| t.0));
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
                    TypeExpr::Generic { params, .. } => Some(params),
                    _ => None,
                };
                if let Some(params) = params {
                    for (_, kind) in params {
                        if let ast::parameter::ParameterKind::Tagged(sp) = kind
                            && let TypeExpr::Nominal(name, span) = &sp.value
                            && name.as_str().is_capitalized_type_name()
                            && !type_scope.contains(name)
                        {
                            push_flaw(
                                *span,
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
                        if let TypeExpr::Nominal(name, span) = &pattern.value
                            && name.as_str().is_capitalized_type_name()
                            && !type_scope.contains(name)
                        {
                            push_flaw(
                                *span,
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
        let ast::HasMember::Function(function) = member else {
            continue;
        };
        let Some(qualifier) = &function.qualifier else {
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
    match &expr.value {
        Expr::SelfRef => spans.push(expr.span_id),
        Expr::Loop(loop_expr) => match loop_expr {
            ast::Loop::While(while_loop) => {
                collect_self_ref_spans(&while_loop.cond, spans);
                for expr in &while_loop.exprs {
                    collect_self_ref_spans(expr, spans);
                }
            }
            ast::Loop::ForIn(for_in) => {
                collect_self_ref_spans(&for_in.iter, spans);
                for expr in &for_in.exprs {
                    collect_self_ref_spans(expr, spans);
                }
            }
        },
        Expr::Binary(binary) => {
            collect_self_ref_spans(&binary.lhs, spans);
            collect_self_ref_spans(&binary.rhs, spans);
        }
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    collect_self_ref_spans(arg, spans);
                }
            }
        }
        Expr::FormatString(format_string) => {
            for part in &format_string.parts {
                if let ast::FormatPart::Expr(expr, _) = part {
                    collect_self_ref_spans(expr, spans);
                }
            }
        }
        Expr::Range(range) => {
            collect_self_ref_spans(&range.start, spans);
            collect_self_ref_spans(&range.end, spans);
        }
        Expr::Bind(bind) => match &bind.value {
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
        },
        Expr::When(when) => {
            if let Some(subject) = &when.subject {
                collect_self_ref_spans(subject, spans);
            }
            for arm in &when.arms {
                match arm {
                    ast::WhenArm::Cond {
                        condition, body, ..
                    } => {
                        collect_self_ref_spans(condition, spans);
                        collect_self_ref_spans(body, spans);
                    }
                    ast::WhenArm::Is { body, .. } | ast::WhenArm::Else(body, _) => {
                        collect_self_ref_spans(body, spans);
                    }
                }
            }
        }
        Expr::If(if_expr) => {
            collect_self_ref_spans(&if_expr.subject, spans);
            for expr in &if_expr.body {
                collect_self_ref_spans(expr, spans);
            }
            if let Some(expr) = &if_expr.ret.value {
                collect_self_ref_spans(expr, spans);
            }
        }
        Expr::TagCall(call) => {
            for arg in &call.args {
                collect_self_ref_spans(arg, spans);
            }
        }
        Expr::TupleAlloc { init, size } => {
            collect_self_ref_spans(init, spans);
            collect_self_ref_spans(size, spans);
        }
        Expr::TupleGet { base, .. } | Expr::RecordGet { base, .. } => {
            collect_self_ref_spans(base, spans);
        }
        Expr::TupleSet { base, value, .. } | Expr::RecordSet { base, value, .. } => {
            collect_self_ref_spans(base, spans);
            collect_self_ref_spans(value, spans);
        }
        Expr::Cast { expr, .. }
        | Expr::TakePtr(expr)
        | Expr::Ref { inner: expr, .. }
        | Expr::ConsumeArg(expr)
        | Expr::Eat(expr)
        | Expr::Deref(expr)
        | Expr::Negate(expr) => collect_self_ref_spans(expr, spans),
        Expr::BufGet { buf, index } => {
            collect_self_ref_spans(buf, spans);
            collect_self_ref_spans(index, spans);
        }
        Expr::BufSet { buf, index, value } => {
            collect_self_ref_spans(buf, spans);
            collect_self_ref_spans(index, spans);
            collect_self_ref_spans(value, spans);
        }
        Expr::RecordLit(fields) => {
            for (_, expr) in fields {
                collect_self_ref_spans(expr, spans);
            }
        }
        Expr::TupleLit(exprs) | Expr::List(exprs) => {
            for expr in exprs {
                collect_self_ref_spans(expr, spans);
            }
        }
        Expr::Destructure { value, .. } => collect_self_ref_spans(value, spans),
        Expr::Lit(_)
        | Expr::AnonymousTag(_)
        | Expr::TypeNominal(_)
        | Expr::TypeInRange(_)
        | Expr::TypeQualified(_)
        | Expr::TypeGeneric { .. }
        | Expr::TypeRef { .. }
        | Expr::Asm(_) => {}
    }
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
        let ParameterKind::Tagged(sp) = kind else {
            continue;
        };
        let TypeExpr::InRange {
            bounds: ast::InRangeBounds::Tag(tag),
            span,
        } = &sp.value
        else {
            continue;
        };
        if tag_types
            .get(&TagId(*tag))
            .is_some_and(|ty| ty.is_bounded_int())
        {
            flaws.push((
                *span,
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

fn resolve_tag_pass(
    tag_name: &Intern<String>,
    declare: &ast::declare::Declare,
    all_tags: &ast::TagMap,
    tag_params: &HashMap<Intern<String>, Parameters>,
    cross_file_tag_types: &HashMap<TagId, Ty>,
    resolved: &HashMap<Intern<String>, Ty>,
) -> Ty {
    // Do not feed a tag's previous-pass type back into its own resolution.
    // Recursive declarations such as `Type is ... Tuple(List(Type)) ...` would
    // otherwise expand one full copy of `Type` per fixpoint pass, ballooning
    // memory before the pass cap is reached. Self references should remain
    // nominal/opaque at this stage; consumers can still identify them by name.
    let mut resolved_without_self = resolved.clone();
    resolved_without_self.remove(tag_name);
    resolve_declare_value(
        tag_name,
        &declare.value,
        all_tags,
        all_tags,
        tag_params,
        cross_file_tag_types,
        &resolved_without_self,
    )
}

pub(crate) fn resolve_tags_fixpoint(
    tags: &ast::TagMap,
    tag_params: &HashMap<Intern<String>, Parameters>,
    cross_file_tag_types: &HashMap<TagId, Ty>,
    max_passes: usize,
) -> HashMap<Intern<String>, Ty> {
    let mut resolved: HashMap<Intern<String>, Ty> = HashMap::new();
    for _pass in 0..max_passes {
        let mut changed = false;
        for (name, declare) in tags {
            let new_ty = resolve_tag_pass(
                name,
                declare,
                tags,
                tag_params,
                cross_file_tag_types,
                &resolved,
            );
            match resolved.get(name) {
                Some(existing) if existing == &new_ty => {}
                _ => {
                    resolved.insert(*name, new_ty);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    resolved
}

pub(crate) fn rebuild_own_variant_map(
    variant_map: &mut VariantMap,
    tag_types: &HashMap<TagId, Ty>,
    own_unions: &HashSet<Intern<String>>,
) {
    variant_map.retain(|_, _| false);
    for (tag_id, ty) in tag_types {
        if own_unions.contains(&tag_id.0) {
            push_union_variants_to_map(variant_map, tag_id.0, ty);
        }
    }
}

fn push_union_variants_to_map(
    variant_map: &mut VariantMap,
    union_name: Intern<String>,
    resolved_ty: &Ty,
) {
    match resolved_ty {
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

fn literal_const_from_type_expr(expr: &TypeExpr) -> Option<ConstValue> {
    match expr {
        TypeExpr::Literal(Literal::String(s), _) => Some(ConstValue::String(s.clone())),
        TypeExpr::Literal(Literal::Int(n), _) => Some(ConstValue::Int(*n as i128)),
        TypeExpr::Literal(Literal::Number(n), _) => Some(ConstValue::Int(*n as i128)),
        TypeExpr::Literal(Literal::Float(HashFloat(f)), _) => {
            Some(ConstValue::Float(HashFloat(*f)))
        }
        _ => None,
    }
}

fn resolve_declare_value(
    declaring_name: &Intern<String>,
    value: &DeclareValue,
    all_tags: &ast::TagMap,
    _own_tags: &ast::TagMap,
    tag_params: &HashMap<Intern<String>, Parameters>,
    cross_file_tag_types: &HashMap<TagId, Ty>,
    resolved: &HashMap<Intern<String>, Ty>,
) -> Ty {
    let tag_types = resolved_types(resolved, cross_file_tag_types);
    let env = TypeEnv::new(&tag_types)
        .with_tag_params(tag_params)
        .with_tag_decls(all_tags);

    match value {
        DeclareValue::Alias(sp) => env.resolve(&sp.value),
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
                        TypeExpr::Generic { name, params, .. } => {
                            let fields: Vec<(Intern<String>, Box<Ty>)> = params
                                .iter()
                                .map(|(name, kind)| {
                                    let ty = match kind {
                                        ParameterKind::Tagged(sp) => env.resolve(&sp.value),
                                        ParameterKind::ValueParam { ty }
                                        | ParameterKind::Inferred { ty } => env.resolve(&ty.value),
                                        ParameterKind::Generic => Ty::Opaque(*name),
                                        ParameterKind::Default(expr) => {
                                            if let Some(te) = expr.value.as_type_expr() {
                                                env.resolve(&te)
                                            } else {
                                                Ty::i64()
                                            }
                                        }
                                    };
                                    (*name, Box::new(ty))
                                })
                                .collect();
                            (*name, fields)
                        }
                        TypeExpr::Nominal(name, _) => {
                            (*name, Vec::<(Intern<String>, Box<Ty>)>::new())
                        }
                        _ => (
                            Intern::new(String::new()),
                            Vec::<(Intern<String>, Box<Ty>)>::new(),
                        ),
                    };
                    let result_ty = v.result_ty().map(|rt| env.resolve(&rt.value));
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
        DeclareValue::Range(start, end) => {
            Ty::range_value_record(Intern::new("range_value".to_string()), *start, *end)
        }
        DeclareValue::Set() => Ty::Unit,
        DeclareValue::Has(members) => {
            let resolved_fields: Vec<(Intern<String>, Box<Ty>)> = members
                .iter()
                .filter_map(|m| match m {
                    HasMember::Property(p) => {
                        let field_ty =
                            p.ty.as_ref()
                                .map(|ty| env.resolve(&ty.value))
                                .unwrap_or(Ty::Unit);
                        Some((p.name, Box::new(field_ty)))
                    }
                    HasMember::Function(_) => None,
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
                if let Some(subject_te) = subject.value.as_type_expr() {
                    let subject_ty = env.resolve(&subject_te);
                    // Try to simplify via compile-time constant folding
                    for arm in &when.arms {
                        if let WhenArm::Is { body, .. } = arm
                            && let Some(body_te) = body.value.as_type_expr()
                        {
                            let body_ty = env.resolve(&body_te);
                            return body_ty;
                        }
                    }
                    subject_ty
                } else {
                    Ty::Unit
                }
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

            let referent_type = reference_referent_type(param_type);
            let group_name = bind.param_groups.get(param_name).cloned();
            if let Some(group_name) = group_name.as_ref()
                && let Some(group_id) = named.get(group_name).copied()
            {
                if groups[group_id.0 as usize].referent_type != *referent_type {
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
        match resolve_group_path(root_type, path) {
            Ok((projections, resolved_type)) => {
                group.projections = Some(projections);
                if resolved_type != group.referent_type {
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
            TypeExpr::Ref {
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
            TypeExpr::Ref { group: None, .. } => match groups.len() {
                1 => Some(GroupId(0)),
                0 => {
                    flaws.push((
                        spanned.span_id,
                        Diagnostic::new(
                            "type-reference-return-without-target-group",
                            "reference return has no eligible reference input group",
                        ),
                    ));
                    None
                }
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
    InvalidProjection { segment: Intern<String>, ty: Ty },
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

    fn unwrapped(self) -> Self {
        match self {
            Self::Type(Ty::Ref { inner, .. }) => Self::Type(*inner).unwrapped(),
            cursor => cursor,
        }
    }
}

fn reference_referent_type(ty: &Ty) -> &Ty {
    match ty {
        Ty::Ref { inner, .. } => reference_referent_type(inner),
        ty => ty,
    }
}

fn resolve_group_path(
    root_type: &Ty,
    path: &ast::GroupPath,
) -> Result<(Vec<GroupProjection>, Ty), GroupPathError> {
    let mut cursor = GroupPathCursor::Type(root_type.clone());
    let mut projections = Vec::with_capacity(path.segments.len());
    for segment in &path.segments {
        cursor = cursor.unwrapped();
        match segment.as_str() {
            "items" => match cursor {
                GroupPathCursor::Type(Ty::Array { elem, .. }) => {
                    projections.push(GroupProjection::Items);
                    cursor = GroupPathCursor::Type(*elem);
                }
                cursor => {
                    return Err(GroupPathError::InvalidProjection {
                        segment: *segment,
                        ty: cursor.into_type(),
                    });
                }
            },
            "pointee" => match cursor {
                GroupPathCursor::Type(Ty::Ptr { inner }) => {
                    projections.push(GroupProjection::Pointee);
                    cursor = GroupPathCursor::Type(*inner);
                }
                cursor => {
                    return Err(GroupPathError::InvalidProjection {
                        segment: *segment,
                        ty: cursor.into_type(),
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
                            ty: cursor.into_type(),
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
                        ty: cursor.into_type(),
                    });
                }
            },
        }
    }
    Ok((projections, cursor.unwrapped().into_type()))
}

fn resolve_return_type(
    bind: &Bind,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: &ast::TagMap,
    dependent_binder: BinderId,
) -> Ty {
    if let Some(sp) = &bind.return_tag {
        let ty = TypeEnv::new(tag_types)
            .with_tag_params(tag_params)
            .with_tag_decls(tag_decls)
            .with_dependent_binder(dependent_binder)
            .resolve(&sp.value);
        return nominalize_explicit_ty_surface(&sp.value, ty);
    }
    if let Some(name) = &bind.return_type_name {
        let ty = tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name));
        return nominalize_explicit_ty_surface(&TypeExpr::Nominal(*name, SpanId::INVALID), ty);
    }
    // Try to infer from body
    match &bind.value {
        BindValue::Expr(e) => resolve_type_from_typed_expr(e, tag_types),
        BindValue::Body { ret, .. } => ret
            .value
            .as_ref()
            .map(|e| resolve_type_from_typed_expr(e, tag_types))
            .unwrap_or(Ty::Unit),
        _ => Ty::Unit,
    }
}

fn resolve_param_types(
    bind: &Bind,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: &ast::TagMap,
    dependent_binder: BinderId,
) -> Vec<(Intern<String>, Ty)> {
    let Some(params) = &bind.params else {
        return Vec::new();
    };
    params
        .iter()
        .map(|(name, kind)| {
            let ty = match kind {
                ParameterKind::Tagged(sp) => TypeEnv::new(tag_types)
                    .with_tag_params(tag_params)
                    .with_tag_decls(tag_decls)
                    .with_dependent_binder(dependent_binder)
                    .resolve(&sp.value),
                ParameterKind::ValueParam { ty } | ParameterKind::Inferred { ty } => {
                    TypeEnv::new(tag_types)
                        .with_tag_params(tag_params)
                        .with_tag_decls(tag_decls)
                        .with_dependent_binder(dependent_binder)
                        .resolve(&ty.value)
                }
                ParameterKind::Generic => Ty::Opaque(*name),
                ParameterKind::Default(expr) => {
                    if let Some(te) = expr.value.as_type_expr() {
                        TypeEnv::new(tag_types)
                            .with_tag_params(tag_params)
                            .with_tag_decls(tag_decls)
                            .with_dependent_binder(dependent_binder)
                            .resolve(&te)
                    } else {
                        Ty::i64()
                    }
                }
            };
            (*name, ty)
        })
        .collect()
}

fn is_type_param_marker(ty: &TypeExpr) -> bool {
    matches!(ty, TypeExpr::Nominal(name, _) if name.as_str() == "Type")
}

fn param_kind_from_parameter_kind(
    kind: &ParameterKind,
    tag_types: &HashMap<Intern<String>, Ty>,
) -> ParamKind {
    match kind {
        ParameterKind::Generic => ParamKind::Type,
        ParameterKind::Tagged(sp) if is_type_param_marker(&sp.value) => ParamKind::Type,
        ParameterKind::Tagged(sp) => {
            let ty = TypeEnv::new(tag_types).resolve(&sp.value);
            ParamKind::Value(Box::new(ty))
        }
        ParameterKind::ValueParam { ty } | ParameterKind::Inferred { ty } => {
            let ty = TypeEnv::new(tag_types).resolve(&ty.value);
            ParamKind::Value(Box::new(ty))
        }
        ParameterKind::Default(expr) => {
            if let Some(te) = expr.value.as_type_expr()
                && te.is_type_surface()
            {
                let ty = TypeEnv::new(tag_types).resolve(&te);
                ParamKind::Value(Box::new(ty))
            } else {
                ParamKind::Type
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::cursor::TokenCursor;

    fn resolved_args(ty: &Ty) -> &[(Intern<String>, ast::TyArg)] {
        match ty {
            Ty::Record {
                resolved_params: Some(params),
                ..
            } => params,
            other => panic!("expected resolved record parameters, found {other:?}"),
        }
    }

    fn inferred_arg(ty: &Ty, index: usize) -> ast::DependentArgId {
        match &resolved_args(ty)[index].1 {
            ast::TyArg::Const(ast::ConstExpr::Inferred(id)) => *id,
            other => panic!("expected inferred argument, found {other:?}"),
        }
    }

    #[test]
    fn qualified_method_requires_inherited_trait() {
        let ast = TokenCursor::parse_source(
            "TraitA has run(ref self) Int\nCombined has TraitA\n    TraitB.run(ref self) Int: 1\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());

        assert!(typed.declaration_flaws.iter().any(|(_, diagnostic)| {
            diagnostic.code.slug() == "type-uninherited-trait-qualifier"
        }));
    }

    #[test]
    fn qualified_method_must_exist_on_inherited_trait() {
        let ast = TokenCursor::parse_source(
            "TraitA has stop(ref self) Int\nCombined has TraitA\n    TraitA.run(ref self) Int: 1\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());

        assert!(
            typed
                .declaration_flaws
                .iter()
                .any(|(_, diagnostic)| { diagnostic.code.slug() == "type-unknown-trait-method" })
        );
    }

    #[test]
    fn qualified_method_requires_resolvable_trait() {
        let ast =
            TokenCursor::parse_source("Combined has Missing\n    Missing.run(ref self) Int: 1\n");
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());

        assert!(
            typed
                .declaration_flaws
                .iter()
                .any(|(_, diagnostic)| { diagnostic.code.slug() == "type-unknown-trait" })
        );
    }

    #[test]
    fn qualified_method_accepts_transitively_inherited_trait_method() {
        let ast = TokenCursor::parse_source(
            "Base has run(ref self) Int\nMid has Base\nCombined has Mid\n    Base.run(ref self) Int: 1\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());

        assert!(!typed.declaration_flaws.iter().any(|(_, diagnostic)| {
            matches!(
                diagnostic.code.slug(),
                "type-uninherited-trait-qualifier" | "type-unknown-trait-method"
            )
        }));
    }

    #[test]
    fn derived_group_requires_declared_root_group() {
        let ast = TokenCursor::parse_source("bad(mut{missing.items} item x): 0\n");
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());

        assert!(
            typed
                .declaration_flaws
                .iter()
                .any(|(_, diagnostic)| { diagnostic.code.slug() == "type-unknown-target-group" })
        );
    }

    #[test]
    fn resolves_named_field_and_item_group_projections() {
        let ast = TokenCursor::parse_source(
            "Ring has power Int\nEntity has rings Array(Ring, 2)\ninspect(ref{entities} entity Entity, ref{entities.rings.items} ring Ring): 0\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());
        let bind = typed.defs.get(&DefId(Intern::from_ref("inspect"))).unwrap();

        assert_eq!(
            bind.groups[1].projections,
            Some(vec![
                GroupProjection::Field {
                    name: Intern::from_ref("rings"),
                    index: 0,
                },
                GroupProjection::Items,
            ])
        );
        assert!(
            typed.declaration_flaws.is_empty(),
            "{:?}",
            typed.declaration_flaws
        );
    }

    #[test]
    fn target_group_path_rejects_unknown_field() {
        let ast = TokenCursor::parse_source(
            "Ring has power Int\nEntity has rings Array(Ring, 2)\nbad(ref{entities} entity Entity, ref{entities.armor.items} ring Ring): 0\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());

        assert!(typed.declaration_flaws.iter().any(|(_, diagnostic)| {
            diagnostic.code.slug() == "type-unknown-target-group-field"
        }));
    }

    #[test]
    fn target_group_path_rejects_items_on_non_collection() {
        let ast = TokenCursor::parse_source(
            "Ring has power Int\nEntity has ring Ring\nbad(ref{entities} entity Entity, ref{entities.ring.items} ring Ring): 0\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());

        assert!(typed.declaration_flaws.iter().any(|(_, diagnostic)| {
            diagnostic.code.slug() == "type-invalid-target-group-projection"
        }));
    }

    #[test]
    fn target_group_path_rejects_pointee_on_non_pointer() {
        let ast = TokenCursor::parse_source(
            "Ring has power Int\nEntity has ring Ring\nbad(ref{entities} entity Entity, ref{entities.ring.pointee} ring Ring): 0\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());

        assert!(typed.declaration_flaws.iter().any(|(_, diagnostic)| {
            diagnostic.code.slug() == "type-invalid-target-group-projection"
        }));
    }

    #[test]
    fn target_group_path_rejects_referent_type_mismatch() {
        let ast = TokenCursor::parse_source(
            "Ring has power Int\nEntity has rings Array(Ring, 2)\nbad(ref{entities} entity Entity, ref{entities.rings.items} value Int): 0\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());

        assert!(
            typed.declaration_flaws.iter().any(|(_, diagnostic)| {
                diagnostic.code.slug() == "type-incompatible-target-group"
            })
        );
    }

    #[test]
    fn resolves_nested_target_group_path() {
        let ast = TokenCursor::parse_source(
            "Ring has power Int\nEntity has rings Array(Ring, 2)\nWorld has entity Entity\ninspect(ref{world} world World, ref{world.entity.rings.items} ring Ring): 0\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());
        let bind = typed.defs.get(&DefId(Intern::from_ref("inspect"))).unwrap();

        assert_eq!(bind.groups[1].projections.as_ref().unwrap().len(), 3);
        assert!(
            typed.declaration_flaws.is_empty(),
            "{:?}",
            typed.declaration_flaws
        );
    }

    #[test]
    fn resolves_variant_payload_target_group_path() {
        let ast = TokenCursor::parse_source(
            "Ring has power Int\nChoice is Some(value Ring) or None\ninspect(ref{choices} choice Choice, ref{choices.Some.value} ring Ring): 0\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());
        let bind = typed.defs.get(&DefId(Intern::from_ref("inspect"))).unwrap();

        assert_eq!(
            bind.groups[1].projections,
            Some(vec![
                GroupProjection::Variant {
                    name: Intern::from_ref("Some"),
                },
                GroupProjection::Field {
                    name: Intern::from_ref("value"),
                    index: 0,
                },
            ])
        );
        assert!(
            typed.declaration_flaws.is_empty(),
            "{:?}",
            typed.declaration_flaws
        );
    }

    #[test]
    fn infers_ungrouped_reference_return_from_sole_reference_input() {
        let ast = TokenCursor::parse_source("first(ref values List(x)) ref x: values\n");
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());
        let bind = typed.defs.get(&DefId(Intern::from_ref("first"))).unwrap();

        assert_eq!(bind.param_groups, vec![Some(GroupId(0))]);
        assert_eq!(bind.groups.len(), 1);
        assert_eq!(bind.groups[0].path, None);
        assert_eq!(bind.return_group, Some(GroupId(0)));
        assert!(typed.declaration_flaws.is_empty());
    }

    #[test]
    fn reuses_named_reference_group_for_params_and_return() {
        let ast =
            TokenCursor::parse_source("choose(ref{r} left x, ref{r} right x) ref{r} x: left\n");
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());
        let bind = typed.defs.get(&DefId(Intern::from_ref("choose"))).unwrap();

        assert_eq!(bind.param_groups, vec![Some(GroupId(0)), Some(GroupId(0))]);
        assert_eq!(bind.groups.len(), 1);
        assert_eq!(
            bind.groups[0].path,
            Some(ast::GroupPath::root(Intern::from_ref("r")))
        );
        assert_eq!(bind.return_group, Some(GroupId(0)));
        assert!(typed.declaration_flaws.is_empty());
    }

    #[test]
    fn omitted_hidden_record_args_complete_definition_signature() {
        let ast = TokenCursor::parse_source(
            "List(x, length Int: ?, state Int: ?) has value x\nkeep(value List(Int)) List(Int): value\n",
        );
        let typed = stage_declare(&ast, FileId(4), &TransformCtx::new());
        let bind = typed.defs.get(&DefId(Intern::from_ref("keep"))).unwrap();
        let param_ty = &bind.params[0].1;

        assert_eq!(resolved_args(param_ty).len(), 3);
        assert!(matches!(resolved_args(param_ty)[0].1, ast::TyArg::Type(_)));
        assert_eq!(inferred_arg(param_ty, 1).slot, 1);
        assert_eq!(inferred_arg(param_ty, 2).slot, 2);
        assert_eq!(inferred_arg(param_ty, 1).binder, bind.dependent_binder);
        assert_eq!(
            inferred_arg(param_ty, 1),
            inferred_arg(&bind.return_type, 1)
        );
        assert_eq!(
            inferred_arg(param_ty, 2),
            inferred_arg(&bind.return_type, 2)
        );
    }

    #[test]
    fn explicit_hidden_record_args_remain_concrete() {
        let ast = TokenCursor::parse_source(
            "List(x, length Int: ?, state Int: ?) has value x\nkeep(value List(Int, 3, 9)): value\n",
        );
        let typed = stage_declare(&ast, FileId(0), &TransformCtx::new());
        let bind = typed.defs.get(&DefId(Intern::from_ref("keep"))).unwrap();
        let args = resolved_args(&bind.params[0].1);

        assert_eq!(args.len(), 3);
        assert_eq!(
            args[1].1,
            ast::TyArg::Const(ast::ConstExpr::Value(ConstValue::Int(3)))
        );
        assert_eq!(
            args[2].1,
            ast::TyArg::Const(ast::ConstExpr::Value(ConstValue::Int(9)))
        );
    }

    #[test]
    fn hidden_args_are_scoped_by_definition_and_file() {
        let source = "List(x, state Int: ?) has value x\nfirst(value List(Int)): value\nsecond(value List(Int)): value\n";
        let ast = TokenCursor::parse_source(source);
        let typed = stage_declare(&ast, FileId(7), &TransformCtx::new());
        let first = typed.defs.get(&DefId(Intern::from_ref("first"))).unwrap();
        let second = typed.defs.get(&DefId(Intern::from_ref("second"))).unwrap();
        assert_ne!(
            inferred_arg(&first.params[0].1, 1),
            inferred_arg(&second.params[0].1, 1)
        );

        let other_ast = TokenCursor::parse_source(
            "List(x, state Int: ?) has value x\nfirst(value List(Int)): value\n",
        );
        let other = stage_declare(&other_ast, FileId(8), &TransformCtx::new());
        let other_first = other.defs.get(&DefId(Intern::from_ref("first"))).unwrap();
        assert_ne!(
            inferred_arg(&first.params[0].1, 1),
            inferred_arg(&other_first.params[0].1, 1)
        );
    }

    #[test]
    fn callable_signature_transports_dependent_binder() {
        let ast = TokenCursor::parse_source(
            "List(x, state Int: ?) has value x\nkeep(value List(Int)) List(Int): value\n",
        );
        let typed = stage_declare(&ast, FileId(12), &TransformCtx::new());
        let bind = typed.defs.get(&DefId(Intern::from_ref("keep"))).unwrap();
        let ctx = TransformCtx::from_typed_asts(&[&typed]);
        let signature = ctx
            .cross_file_callable_signatures
            .get(&DefId(Intern::from_ref("keep")))
            .unwrap();

        assert_eq!(signature.dependent_binder, bind.dependent_binder);
        assert_eq!(
            inferred_arg(&signature.params[0].1, 1).binder,
            signature.dependent_binder
        );
        assert_eq!(
            inferred_arg(&signature.return_type, 1).binder,
            signature.dependent_binder
        );
    }
}
