//! Stage 1: Declare — Resolve tag/bind declarations, build file-level index.
//!
//! This stage walks the `ParseAst` tags and defs, translates them into
//! [`TypedTag`] and [`TypedBind`], populates `tag_types`, `fn_return_types`,
//! and `variant_map`, and detects declaration-level flaws.

use internment::Intern;
use std::collections::HashMap;

use diagnostic::TypeSymptom;

use super::{ParseAst, TransformCtx};
use crate::analysis::{ConstValue, resolve_type_expr_from_map};
use crate::marker::MarkerBinding;
use crate::prelude::*;
use crate::ty::Ty;
use crate::typed::{DefId, FileId, TagId, TypedBind, TypedFileAst, TypedTag};
use crate::{DeclareValue, TypeExpr};

/// Consumes the `ParseAst` and returns a `TypedFileAst` with declarations
/// resolved and the file-level index populated. The expression arena is left
/// empty — it will be filled in Stage 2.
pub fn stage_declare(parse_ast: &ParseAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    let mut typed = TypedFileAst::new(file_id, parse_ast.span_table.clone());
    typed.resolved_imports = HashMap::new(); // will be populated by import resolution
    typed.marker_registry = ctx.marker_registry.clone();

    // 1. Walk tag declarations and resolve them.
    for (name, declare) in &parse_ast.tags {
        let tag_id = TagId(*name);
        let resolved_ty =
            resolve_declare_value(&declare.value, &parse_ast.tags, &ctx.cross_file_tag_types);

        let typed_tag = TypedTag {
            resolved_ty: resolved_ty.clone(),
            attributes: declare.attributes.clone(),
            marker_bindings: declare.marker_bindings.clone(),
            doc_comment: declare.doc_comment.clone(),
            params: declare.params.clone(),
            declaration_text: declare.to_string(),
        };

        // Check marker bindings against the registry — warn about unrecognized markers.
        for binding in &declare.marker_bindings {
            if !typed.marker_registry.is_recognized(&binding.marker_name) {
                typed.warnings.push(TypeSymptom::UnknownBinding {
                    name: format!(
                        "`{}` is not a recognized marker; recognized markers are `Copy`",
                        binding.marker_name.as_str()
                    ),
                    did_you_mean: None,
                });
            }
        }

        typed.tags.insert(tag_id, typed_tag);
        typed.tag_types.insert(tag_id, resolved_ty.clone());

        // Populate variant_map for union types and const union types.
        match &resolved_ty {
            Ty::Union { variants, .. } => {
                for (variant_name, fields) in variants {
                    let entry = (
                        *name,
                        typed.variant_map.len(),
                        fields
                            .iter()
                            .map(|(fname, fty)| (*fname, *fty.clone()))
                            .collect(),
                    );
                    typed
                        .variant_map
                        .entry(*variant_name)
                        .or_default()
                        .push(entry);
                }
            }
            Ty::ConstUnion { values, .. } => {
                for (i, cv) in values.iter().enumerate() {
                    let vname = cv.display_name();
                    let entry = (*name, i, Vec::new());
                    typed.variant_map.entry(vname).or_default().push(entry);
                }
            }
            _ => {}
        }

        // Record privacy.
        if parse_ast.private_tags.contains(name) {
            typed.private_tags.insert(tag_id);
        }
    }

    // 2. Walk bind declarations and resolve them.
    for (name, bind) in &parse_ast.defs {
        let def_id = DefId(*name);

        let tag_types_raw: HashMap<Intern<String>, Ty> = typed
            .tag_types
            .iter()
            .map(|(tid, ty)| (tid.0, ty.clone()))
            .collect();
        let return_type = resolve_return_type(bind, &tag_types_raw);
        let params = resolve_param_types(bind, &tag_types_raw);

        let typed_bind = TypedBind {
            name: *name,
            name_span: bind.name_span,
            body: crate::typed::BindBody::Expr(crate::typed::ExprId(0)), // placeholder, will be filled in Stage 2
            return_type: return_type.clone(),
            params,
            receiver_type: bind
                .receiver_type
                .as_ref()
                .map(|rt| resolve_type_expr_from_map(&rt.value, &tag_types_raw)),
            attributes: bind.attributes.clone(),
            doc_comment: bind.doc_comment.clone(),
        };

        typed.defs.insert(def_id, typed_bind);
        typed.fn_return_types.insert(def_id, return_type);

        if parse_ast.private_defs.contains(name) {
            typed.private_defs.insert(def_id);
        }
    }

    // Merge cross-file function return types so the typed AST can resolve
    // cross-file function calls during flaw detection.
    for (def_id, return_ty) in &ctx.cross_file_fn_return_types {
        typed
            .fn_return_types
            .entry(*def_id)
            .or_insert_with(|| return_ty.clone());
    }

    // Ensure built-in types are present.
    let str_generic_id = TagId(Intern::new("Str".to_string()));
    typed
        .tag_types
        .entry(str_generic_id)
        .or_insert_with(crate::ty::str_record_ty);

    // Propagate marker bindings from marker-propagating aliases (e.g. `Linear(T)`)
    // through field type expressions in record/union declarations.
    propagate_marker_aliases(&mut typed, parse_ast);

    typed
}

fn resolve_declare_value(
    value: &DeclareValue,
    local_tags: &crate::TagMap,
    cross_file_tag_types: &HashMap<TagId, Ty>,
) -> Ty {
    // Build a combined tag type map from local + cross-file.
    let mut all_tag_types: HashMap<Intern<String>, Ty> = HashMap::new();
    for (name, declare) in local_tags {
        if let Some(ref resolved) = declare.resolved_type {
            all_tag_types.insert(*name, resolved.clone());
        }
    }
    for (tag_id, ty) in cross_file_tag_types {
        all_tag_types.insert(tag_id.0, ty.clone());
    }

    match value {
        DeclareValue::Alias(sp) => resolve_type_expr_from_map(&sp.value, &all_tag_types),
        DeclareValue::Record(params) => {
            let name = local_tags
                .iter()
                .find(|(_, d)| d.value == *value)
                .map(|(n, _)| *n)
                .unwrap_or_else(|| Intern::new("__record__".to_string()));
            let fields: Vec<(Intern<String>, Box<Ty>)> = params
                .iter()
                .map(|(k, kind)| {
                    let ty = match kind {
                        ParameterKind::Tagged(sp) => {
                            if let Some(te) = sp.value.as_type_expr() {
                                resolve_type_expr_from_map(&te, &all_tag_types)
                            } else {
                                Ty::Opaque(*k)
                            }
                        }
                        ParameterKind::Generic => Ty::Opaque(*k),
                        ParameterKind::Default(v) => {
                            resolve_type_from_typed_expr(&v.value, &all_tag_types)
                        }
                    };
                    (*k, Box::new(ty))
                })
                .collect();
            Ty::Record { name, fields }
        }
        DeclareValue::Union { variants } => {
            let name = local_tags
                .iter()
                .find(|(_, d)| d.value == *value)
                .map(|(n, _)| *n)
                .unwrap_or_else(|| Intern::new("__union__".to_string()));

            // Check if all variants are literal values -> ConstUnion.
            let mut lit_values: Vec<ConstValue> = Vec::new();
            let mut lit_base: Option<Ty> = None;
            let mut tag_variants: Vec<(Intern<String>, Vec<(Intern<String>, Box<Ty>)>)> =
                Vec::new();
            let mut all_literals = true;

            for variant in variants {
                let shape = variant.shape();
                if let Some(cv) = const_value_from_type_expr(&shape.value) {
                    let base_ty = base_ty_for_const_value(&cv);
                    if lit_base.is_none() {
                        lit_base = Some(base_ty);
                    }
                    lit_values.push(cv);
                } else {
                    all_literals = false;
                    let (vname, fields) = resolve_variant_shape(&shape.value, &all_tag_types);
                    tag_variants.push((vname, fields));
                }
            }

            if all_literals && !lit_values.is_empty() {
                Ty::ConstUnion {
                    name,
                    base: Box::new(lit_base.unwrap_or(Ty::Int {
                        width: 64,
                        signed: true,
                        value: None,
                    })),
                    values: lit_values,
                }
            } else {
                Ty::Union {
                    name,
                    variants: tag_variants,
                }
            }
        }
        DeclareValue::Range(_start, _end) => Ty::Int {
            width: 64,
            signed: true,
            value: None,
        },
        DeclareValue::InRange(_start, _end) => Ty::Int {
            width: 64,
            signed: true,
            value: None,
        },
        DeclareValue::Set() => Ty::Opaque(Intern::new("Set".to_string())),
    }
}

fn resolve_variant_shape(
    type_expr: &TypeExpr,
    tag_types: &HashMap<Intern<String>, Ty>,
) -> (Intern<String>, Vec<(Intern<String>, Box<Ty>)>) {
    match type_expr {
        TypeExpr::Nominal(name, _) => {
            // Unit variant — no payload fields.
            (*name, vec![])
        }
        TypeExpr::Qualified(path) => {
            let name = path.segments.last().copied().unwrap_or(path.root);
            (name, vec![])
        }
        TypeExpr::Generic { name, params, .. } => {
            let fields: Vec<(Intern<String>, Box<Ty>)> = params
                .iter()
                .map(|(k, kind)| {
                    let ty = match kind {
                        ParameterKind::Tagged(sp) => {
                            if let Some(te) = sp.value.as_type_expr() {
                                resolve_type_expr_from_map(&te, tag_types)
                            } else {
                                Ty::Opaque(*k)
                            }
                        }
                        ParameterKind::Generic => Ty::Opaque(*k),
                        ParameterKind::Default(v) => {
                            resolve_type_from_typed_expr(&v.value, tag_types)
                        }
                    };
                    (*k, Box::new(ty))
                })
                .collect();
            (*name, fields)
        }
        TypeExpr::Literal(_, _) => {
            // ConstUnion variant — no payload fields (value stored as discriminant).
            let name = Intern::new("__const__".to_string());
            (name, vec![])
        }
        TypeExpr::Pointer(_) | TypeExpr::Ref { .. } | TypeExpr::Unit => {
            let name = Intern::new("__unknown__".to_string());
            (name, vec![])
        }
    }
}

/// Try to resolve a type from a typed expression (used for default parameter values).
fn resolve_type_from_typed_expr(expr: &Expr, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    // For default values, we try to infer the type from the expression.
    match expr {
        Expr::Lit(lit) => match lit {
            Literal::Number(_) => Ty::Int {
                width: 64,
                signed: true,
                value: None,
            },
            Literal::Float(_) => Ty::Float { value: None },
            Literal::Int(_) => Ty::Int {
                width: 64,
                signed: false,
                value: None,
            },
            Literal::String(_) => Ty::Opaque(Intern::new("Str".to_string())),
        },
        Expr::TypeNominal(name) => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        Expr::TypeQualified(path) => {
            let last = path.segments.last().copied().unwrap_or(path.root);
            tag_types.get(&last).cloned().unwrap_or(Ty::Opaque(last))
        }
        Expr::TypeGeneric { name, .. } => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        _ => Ty::Opaque(Intern::new("unknown".to_string())),
    }
}

fn resolve_return_type(bind: &Bind, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    if let Some(return_tag) = &bind.return_tag {
        resolve_type_expr_from_map(&return_tag.value, tag_types)
    } else if let Some(return_type_name) = &bind.return_type_name {
        tag_types
            .get(return_type_name)
            .cloned()
            .unwrap_or(Ty::Opaque(*return_type_name))
    } else if let Some(params) = &bind.params {
        if let Some((_, first_kind)) = params.first() {
            match first_kind {
                ParameterKind::Tagged(sp) => {
                    if let Some(te) = sp.value.as_type_expr() {
                        resolve_type_expr_from_map(&te, tag_types)
                    } else {
                        Ty::Opaque(Intern::new("infer".to_string()))
                    }
                }
                _ => Ty::Opaque(Intern::new("infer".to_string())),
            }
        } else {
            Ty::Opaque(Intern::new("infer".to_string()))
        }
    } else {
        Ty::Opaque(Intern::new("infer".to_string()))
    }
}

fn resolve_param_types(
    bind: &Bind,
    tag_types: &HashMap<Intern<String>, Ty>,
) -> Vec<(Intern<String>, Ty)> {
    let Some(params) = &bind.params else {
        return Vec::new();
    };
    params
        .iter()
        .map(|(name, kind)| {
            let ty = match kind {
                ParameterKind::Tagged(sp) => {
                    if let Some(te) = sp.value.as_type_expr() {
                        resolve_type_expr_from_map(&te, tag_types)
                    } else {
                        Ty::Opaque(*name)
                    }
                }
                ParameterKind::Generic => Ty::Opaque(*name),
                ParameterKind::Default(v) => resolve_type_from_typed_expr(&v.value, tag_types),
            };
            (*name, ty)
        })
        .collect()
}

/// Try to extract a ConstValue from a TypeExpr (used for ConstUnion detection).
fn const_value_from_type_expr(e: &TypeExpr) -> Option<ConstValue> {
    match e {
        TypeExpr::Literal(lit, _) => match lit {
            Literal::String(s) => Some(ConstValue::String(s.clone())),
            Literal::Int(n) => Some(ConstValue::Int(*n as i128)),
            Literal::Float(f) => Some(ConstValue::Float(*f)),
            Literal::Number(n) => Some(ConstValue::Int(*n as i128)),
        },
        _ => None,
    }
}

fn base_ty_for_const_value(cv: &ConstValue) -> Ty {
    match cv {
        ConstValue::Int(_) => Ty::Int {
            width: 64,
            signed: true,
            value: None,
        },
        ConstValue::Float(_) => Ty::Float { value: None },
        ConstValue::String(_) => crate::ty::str_record_ty(),
        ConstValue::Tag { .. } => Ty::Opaque(Intern::new("tag".to_string())),
        ConstValue::Record { .. } => Ty::Opaque(Intern::new("Record".to_string())),
        ConstValue::List(_) => Ty::Opaque(Intern::new("List".to_string())),
    }
}

/// After all tags are resolved, propagate marker bindings from marker-propagating
/// aliases (like `Linear(x) is x and is not Copy`) through field type expressions.
///
/// When a record field uses `Linear(Connection)`, the `not Copy` binding from
/// `Linear`'s declaration propagates to the containing tag. This implements
/// field-level linearity: `Database has (conn Linear(Connection))` marks
/// `Database` as not Copy via the `AnyField` inference rule.
fn propagate_marker_aliases(typed: &mut TypedFileAst, parse_ast: &ParseAst) {
    // Find marker-propagating aliases: tags whose marker bindings reference
    // known markers (e.g. Linear registers `and is not Copy`).
    // Check both the current file's tags and the cross-file marker registry.
    let mut marker_aliases: HashMap<Intern<String>, Vec<MarkerBinding>> = HashMap::new();

    // From current file's typed tags
    for (tag_id, tag) in &typed.tags {
        if !tag.marker_bindings.is_empty() {
            marker_aliases.insert(tag_id.0, tag.marker_bindings.clone());
        }
    }

    // From cross-file marker registry: any type with a positive binding for a
    // known marker is a candidate. Build this from the registry's binding sets.
    for marker_name in typed.marker_registry.recognized_markers() {
        if let Some(def) = typed.marker_registry.definition(&marker_name) {
            for type_name in &def.positive_bindings {
                if !marker_aliases.contains_key(type_name) {
                    marker_aliases.insert(
                        *type_name,
                        vec![MarkerBinding {
                            marker_name,
                            positive: true,
                            args: Vec::new(),
                        }],
                    );
                }
            }
        }
    }

    if marker_aliases.is_empty() {
        return;
    }

    // Walk all declarations and check their value expressions for references
    // to marker-propagating aliases.
    for (name, declare) in &parse_ast.tags {
        let tag_id = TagId(*name);
        let extra = collect_alias_markers_from_value(&declare.value, &marker_aliases);
        if !extra.is_empty()
            && let Some(tag) = typed.tags.get_mut(&tag_id)
        {
            tag.marker_bindings.extend(extra);
        }
    }
}

/// Walk a `DeclareValue` and collect marker bindings from any field type
/// expressions that reference marker-propagating aliases.
fn collect_alias_markers_from_value(
    value: &DeclareValue,
    aliases: &HashMap<Intern<String>, Vec<MarkerBinding>>,
) -> Vec<MarkerBinding> {
    match value {
        DeclareValue::Record(params) => {
            let mut bindings = Vec::new();
            for (_name, kind) in params {
                if let ParameterKind::Tagged(sp) = kind
                    && let Some(te) = sp.value.as_type_expr()
                    && let Some(mb) = collect_alias_markers_from_type_expr(&te, aliases)
                {
                    bindings.extend(mb);
                }
            }
            bindings
        }
        DeclareValue::Union { variants } => {
            let mut bindings = Vec::new();
            for variant in variants {
                let shape = variant.shape();
                if let Some(mb) = collect_alias_markers_from_type_expr(&shape.value, aliases) {
                    bindings.extend(mb);
                }
            }
            bindings
        }
        DeclareValue::Alias(sp) => {
            collect_alias_markers_from_type_expr(&sp.value, aliases).unwrap_or_default()
        }
        _ => vec![],
    }
}

/// Check a `TypeExpr` for references to marker-propagating aliases.
///
/// For `Generic { name: "Linear", ... }`, checks if `Linear` is a marker
/// alias and returns its marker bindings.
fn collect_alias_markers_from_type_expr(
    te: &TypeExpr,
    aliases: &HashMap<Intern<String>, Vec<MarkerBinding>>,
) -> Option<Vec<MarkerBinding>> {
    match te {
        TypeExpr::Generic { name, params, .. } => {
            // Check if the generic itself is a marker alias: Linear(Connection)
            if let Some(bindings) = aliases.get(name) {
                return Some(bindings.clone());
            }
            // Check inner type parameters for marker aliases
            for (_, kind) in params {
                if let ParameterKind::Tagged(sp) = kind
                    && let Some(inner_te) = sp.value.as_type_expr()
                    && let Some(inner_bindings) =
                        collect_alias_markers_from_type_expr(&inner_te, aliases)
                {
                    return Some(inner_bindings);
                }
            }
            None
        }
        TypeExpr::Nominal(name, _) => aliases.get(name).cloned(),
        _ => None,
    }
}
