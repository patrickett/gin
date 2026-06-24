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
use crate::analysis::{resolve_type_expr_from_map, resolve_type_expr_with_subst_opts};
use crate::compile_time_trait::{RESERVED_TRAITS, synthesize_reflectable_trait};
use crate::ty::{Ty, UnionVariant};
use crate::typed::{
    DefId, FileId, ResolvedImport, TagId, TypedBind, TypedFileAst, TypedTag, VariantMap,
};
use ast::parameter::Parameters;
use ast::prelude::*;
use ast::type_decl::TypeNameExt;
use ast::{BindValue, ConstValue, DeclareValue, HashFloat, ImportSource, Literal, TypeExpr};

use ast_format::declare::{BindFormatExt, DeclareFormatExt};
use ast_format::type_expr::TypeExprFormatExt;

/// Takes `&FileAst` and returns a `TypedFileAst` with declarations
/// resolved and the file-level index populated. The expression arena is left
/// empty — it will be filled in Stage 2.
pub fn stage_declare(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    let mut typed = TypedFileAst::new(file_id, file_ast.span_table.clone());
    typed.resolved_imports = populate_resolved_imports(file_ast, ctx);
    // Collect raw ModPaths from imports for module path hover detection.
    for import in &file_ast.uses {
        for mi in &import.0 {
            if let ast::ImportSource::Package(mp) = &mi.source {
                typed.import_mod_paths.push(mp.clone());
            }
        }
    }
    typed.blanket_impls = ctx.package_blanket_impls.clone();
    typed.imported_trait_names = file_ast.imported_trait_names();
    for b in &ctx.package_blanket_impls {
        typed.imported_trait_names.insert(b.trait_name);
    }
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
        // Reflectable.shape materializes the whole `Type` graph; skip for IDE package transforms.
        if !ctx.ide_package && name.as_str() != crate::compile_time_trait::REFLECTABLE_TRAIT {
            provided_traits.push(synthesize_reflectable_trait(&resolved_ty));
        }

        // Extract field/method type annotations from `has` bodies.
        let (record_field_types, record_field_docs) = match &declare.value {
            DeclareValue::Interface(members) => {
                let mut map = HashMap::new();
                let mut doc_map = HashMap::new();
                for m in members {
                    // Store just the return/error type surface (without method name),
                    // matching how record fields store `field_name → type_annotation`.
                    // The hover code prepends the word/name itself.
                    use std::fmt::Write;
                    let mut ty_surface = String::new();
                    if !m.params.is_empty() {
                        ty_surface.push('(');
                        let mut first = true;
                        for (param_name, kind) in &m.params {
                            if !first {
                                ty_surface.push_str(", ");
                            }
                            first = false;
                            if let Some(conv) = m.conventions.get(param_name) {
                                match conv {
                                    ast::ParamConvention::Ref(false) => ty_surface.push_str("ref "),
                                    ast::ParamConvention::Ref(true) => ty_surface.push_str("mut "),
                                    ast::ParamConvention::Eat => ty_surface.push_str("eat "),
                                    ast::ParamConvention::Inferred => {}
                                }
                            }
                            let _ = write!(&mut ty_surface, "{}", param_name.as_str());
                            match kind {
                                ast::ParameterKind::Tagged(sp) => {
                                    ty_surface.push(' ');
                                    ty_surface.push_str(&sp.value.format_surface());
                                }
                                ast::ParameterKind::Default(expr) => {
                                    let _ = write!(&mut ty_surface, ": {:?}", expr);
                                }
                                ast::ParameterKind::Generic => {}
                            }
                        }
                        ty_surface.push(')');
                        // Space before return type when there are params
                        if m.return_ty.is_some() {
                            ty_surface.push(' ');
                        }
                    }
                    if let Some(rt) = &m.return_ty {
                        ty_surface.push_str(&rt.value.format_surface());
                    }
                    if let Some(et) = &m.error_ty {
                        ty_surface.push_str(" or ");
                        ty_surface.push_str(&et.value.format_surface());
                    }
                    if ty_surface.is_empty() {
                        ty_surface.push_str("()");
                    }
                    map.insert(m.name, ty_surface);
                    // Collect doc comment
                    if let Some(doc) = &m.doc_comment
                        && !doc.value.is_empty()
                    {
                        doc_map.insert(m.name, doc.value.clone());
                    }
                }
                (map, doc_map)
            }
            _ => (HashMap::new(), HashMap::new()),
        };

        let typed_tag = TypedTag {
            name_span: declare.name_span,
            span: declare.span,
            resolved_ty: resolved_ty.clone(),
            attributes: declare.attributes.clone(),
            doc_comment: declare.doc_comment.clone(),
            record_field_docs,
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
        let tag_types_by_name: HashMap<Intern<String>, Ty> = typed
            .tag_types
            .iter()
            .map(|(id, ty)| (id.0, ty.clone()))
            .collect();
        let return_ty = resolve_return_type(
            declare,
            &tag_types_by_name,
            &tag_params,
            &ctx.cross_file_tag_types,
        );
        let param_types = resolve_param_types(
            declare,
            &tag_types_by_name,
            &tag_params,
            &ctx.cross_file_tag_types,
        );

        let typed_bind = TypedBind {
            name: *name,
            name_span: declare.name_span,
            is_compile_time: declare.is_compile_time,
            return_type: return_ty.clone(),
            declared_return_type: return_ty.clone(),
            params: param_types,
            receiver_type: None,
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

fn check_declare_value_type_scope(
    declare_value: &DeclareValue,
    type_scope: &HashSet<Intern<String>>,
    push_flaw: &mut dyn FnMut(SpanId, Diagnostic),
) {
    check_union_variant_shape_type_scope(declare_value, type_scope, push_flaw);
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
            for (disc, (vname, vfields)) in variants.iter().enumerate() {
                let fields: Vec<(Intern<String>, Ty)> =
                    vfields.iter().map(|(n, t)| (*n, *t.clone())).collect();
                variant_map
                    .entry(*vname)
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
    match value {
        DeclareValue::Alias(sp) => resolve_type_expr_with_subst_opts(
            &sp.value,
            &resolved_types(resolved, cross_file_tag_types),
            &HashMap::new(),
            Some(tag_params),
            Some(all_tags),
        ),
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
                    match &shape.value {
                        TypeExpr::Generic { name, params, .. } => {
                            let fields: Vec<(Intern<String>, Box<Ty>)> = params
                                .iter()
                                .map(|(name, kind)| {
                                    let ty = match kind {
                                        ParameterKind::Tagged(sp) => {
                                            resolve_type_expr_with_subst_opts(
                                                &sp.value,
                                                &resolved_types(resolved, cross_file_tag_types),
                                                &HashMap::new(),
                                                Some(tag_params),
                                                Some(all_tags),
                                            )
                                        }
                                        ParameterKind::Generic => Ty::Opaque(*name),
                                        ParameterKind::Default(expr) => {
                                            if let Some(te) = expr.value.as_type_expr() {
                                                resolve_type_expr_with_subst_opts(
                                                    &te,
                                                    &resolved_types(resolved, cross_file_tag_types),
                                                    &HashMap::new(),
                                                    Some(tag_params),
                                                    Some(all_tags),
                                                )
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
                    }
                })
                .collect();
            Ty::Union {
                name: *declaring_name,
                variants: resolved_variants,
                literal_values: None,
            }
        }

        DeclareValue::InRange(start, end) => Ty::bounded_int(*start, *end),
        DeclareValue::Range(start, end) => {
            Ty::range_value_record(Intern::new("range_value".to_string()), *start, *end)
        }
        DeclareValue::Set() => Ty::Unit,
        DeclareValue::Interface(members) => {
            // Resolve interface members as record fields with function-typed shapes.
            // Each method signature becomes a field whose type is the return type
            // (or `Ty::Unit` if no return type). In a future phase this can become
            // a dedicated `Ty::Interface` with richer method metadata.
            let resolved_fields: Vec<(Intern<String>, Box<Ty>)> = members
                .iter()
                .map(|m| {
                    let field_ty = m
                        .return_ty
                        .as_ref()
                        .map(|rt| {
                            resolve_type_expr_with_subst_opts(
                                &rt.value,
                                &resolved_types(resolved, cross_file_tag_types),
                                &HashMap::new(),
                                Some(tag_params),
                                Some(all_tags),
                            )
                        })
                        // TODO: if return_ty is None and the member name matches a
                        // constructor parameter of the declaring tag (e.g. `allocator` in
                        // `RawList(x, allocator: GlobalAllocator) has (..., allocator,)`),
                        // infer the field type from that parameter's type annotation or
                        // default-value type instead of falling back to Ty::Unit.
                        //
                        // This is like shorthand object syntax
                        .unwrap_or(Ty::Unit);
                    (m.name, Box::new(field_ty))
                })
                .collect();
            Ty::Record {
                name: *declaring_name,
                fields: resolved_fields,
            }
        }
        DeclareValue::When(when) => {
            if let Some(subject) = &when.subject {
                if let Some(subject_te) = subject.value.as_type_expr() {
                    let subject_ty = resolve_type_expr_with_subst_opts(
                        &subject_te,
                        &resolved_types(resolved, cross_file_tag_types),
                        &HashMap::new(),
                        Some(tag_params),
                        Some(all_tags),
                    );
                    // Try to simplify via compile-time constant folding
                    for arm in &when.arms {
                        if let WhenArm::Is { body, .. } = arm
                            && let Some(body_te) = body.value.as_type_expr()
                        {
                            let body_ty = resolve_type_expr_with_subst_opts(
                                &body_te,
                                &resolved_types(resolved, cross_file_tag_types),
                                &HashMap::new(),
                                Some(tag_params),
                                Some(all_tags),
                            );
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

fn resolve_return_type(
    bind: &Bind,
    tag_types: &HashMap<Intern<String>, Ty>,
    _tag_params: &HashMap<Intern<String>, Parameters>,
    _cross_file: &HashMap<TagId, Ty>,
) -> Ty {
    if let Some(sp) = &bind.return_tag {
        let ty = resolve_type_expr_from_map(&sp.value, tag_types);
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
    _tag_params: &HashMap<Intern<String>, Parameters>,
    _cross_file: &HashMap<TagId, Ty>,
) -> Vec<(Intern<String>, Ty)> {
    let Some(params) = &bind.params else {
        return Vec::new();
    };
    params
        .iter()
        .map(|(name, kind)| {
            let ty = match kind {
                ParameterKind::Tagged(sp) => resolve_type_expr_from_map(&sp.value, tag_types),
                ParameterKind::Generic => Ty::Opaque(*name),
                ParameterKind::Default(expr) => {
                    if let Some(te) = expr.value.as_type_expr() {
                        resolve_type_expr_from_map(&te, tag_types)
                    } else {
                        Ty::i64()
                    }
                }
            };
            (*name, ty)
        })
        .collect()
}

fn populate_resolved_imports(
    _file_ast: &FileAst,
    _ctx: &TransformCtx,
) -> HashMap<Intern<String>, ResolvedImport> {
    HashMap::new()
}
