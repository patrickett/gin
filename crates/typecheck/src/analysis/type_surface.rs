//! Raw type resolution — translating AST type declarations into resolved [`Ty`] values.
//!
//! These functions operate on the raw [`DeclareValue`] tree without a resolved
//! `HashMap`. Resolution unfolds aliases, computes union variants and const-unions,
//! and produces the canonical [`Ty`] that other passes consume.

use crate::ty::Ty;
use ast::{DeclareValue, FnCall, InRangeBounds, ParameterKind, Parameters, TypeExpr};
use internment::Intern;
use std::collections::HashMap;

pub fn resolve_type_expr_from_map(e: &TypeExpr, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    let empty: HashMap<Intern<String>, Ty> = HashMap::new();
    resolve_type_expr_with_subst(e, tag_types, &empty, None)
}

fn merge_subst(
    outer: &HashMap<Intern<String>, Ty>,
    local: &HashMap<Intern<String>, Ty>,
) -> HashMap<Intern<String>, Ty> {
    let mut merged = outer.clone();
    merged.extend(local.iter().map(|(k, v)| (*k, v.clone())));
    merged
}

/// Resolve a type-surface [`TypeExpr`] to a [`Ty`], substituting any type-variable
/// names found in `subst`.
///
/// `subst` maps method-scoped type variable names (e.g. `x` in
/// `Range[x].new(start x, end x) Range[x]`) to the `Ty` they currently stand for.
/// During typechecking of the method body itself, the substitution is the identity
/// (`x -> Ty::Opaque(x)`) so the same opaque tag flows through params, body, and
/// return type. At call sites, `subst` can bind `x` to a concrete type (e.g.
/// `Ty::Int`) to instantiate the signature.
///
/// `tag_params` maps tag names to their declaration parameters. Used to fill
/// default type arguments (e.g. `a: LibcAllocator` in `Box(x, a: LibcAllocator)`)
/// when fewer args are provided at the use site.
pub fn resolve_type_expr_with_subst(
    e: &TypeExpr,
    tag_types: &HashMap<Intern<String>, Ty>,
    subst: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> Ty {
    resolve_type_expr_with_subst_opts(e, tag_types, subst, tag_params, None)
}

/// Like [`resolve_type_expr_with_subst`] but can rebuild generic record tags (e.g. `List(NamedTy)`)
/// from declaration field surfaces so nested params like `Pointer(x)` pick up substitutions.
pub fn resolve_type_expr_with_subst_opts(
    e: &TypeExpr,
    tag_types: &HashMap<Intern<String>, Ty>,
    subst: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
    tag_decls: Option<&ast::TagMap>,
) -> Ty {
    match e {
        TypeExpr::Nominal(name, _) => {
            if let Some(t) = subst.get(name) {
                return t.clone();
            }
            // Unresolved lowercase names are type variables (e.g. `x` in `Linear(x) is x`).
            // Return Opaque so generic substitution can replace them.
            if let Some(c) = name.as_str().chars().next()
                && c.is_ascii_lowercase()
            {
                return Ty::Opaque(*name);
            }
            tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name))
        }
        TypeExpr::Generic { name, params, .. } => {
            if params
                .iter()
                .all(|(_, k)| matches!(k, ParameterKind::Generic))
                && params.len() == 1
                && let Some((var, _)) = params.first()
                && let Some(ty) = subst.get(var)
            {
                return ty.clone();
            }
            let local_subst =
                build_subst_for_generic(params, tag_types, subst, tag_params, name, tag_decls);
            let merged_subst = merge_subst(subst, &local_subst);
            if !local_subst.is_empty()
                && let Some(ast::DeclareValue::Interface(members)) =
                    tag_decls.and_then(|tags| tags.get(name)).map(|d| &d.value)
            {
                let fields: Vec<(Intern<String>, Box<Ty>)> = members
                    .iter()
                    .map(|m| {
                        let ty = m
                            .return_ty
                            .as_ref()
                            .map(|rt| {
                                resolve_type_expr_with_subst_opts(
                                    &rt.value,
                                    tag_types,
                                    &merged_subst,
                                    tag_params,
                                    tag_decls,
                                )
                            })
                            .unwrap_or(Ty::Unit);
                        (m.name, Box::new(ty))
                    })
                    .collect();
                return Ty::Record {
                    name: *name,
                    fields,
                };
            }
            if params.is_empty()
                && let Some(ast::DeclareValue::Alias(sp)) =
                    tag_decls.and_then(|tags| tags.get(name)).map(|d| &d.value)
            {
                return resolve_type_expr_with_subst_opts(
                    &sp.value,
                    tag_types,
                    &merged_subst,
                    tag_params,
                    tag_decls,
                );
            }
            let base = tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name));
            if merged_subst.is_empty() {
                base
            } else {
                base.substitute(&merged_subst)
            }
        }
        TypeExpr::Qualified(path) => tag_types
            .get(&path.root)
            .cloned()
            .unwrap_or(Ty::Opaque(path.root)),
        TypeExpr::Literal(..) => Ty::Opaque(Intern::new(String::new())),
        TypeExpr::Pointer(inner) => Ty::Ptr {
            inner: Box::new(resolve_type_expr_with_subst_opts(
                &inner.value,
                tag_types,
                subst,
                tag_params,
                tag_decls,
            )),
        },
        TypeExpr::Ref { inner, mutable } => Ty::Ref {
            inner: Box::new(resolve_type_expr_with_subst_opts(
                &inner.value,
                tag_types,
                subst,
                tag_params,
                tag_decls,
            )),
            mutable: *mutable,
        },

        TypeExpr::Unit => Ty::Unit,
        TypeExpr::InRange { bounds, .. } => resolve_in_range_bounds(bounds, tag_types),
        TypeExpr::ListEmpty | TypeExpr::ListCons { .. } => Ty::Unit,
        TypeExpr::Tuple(elems) => Ty::Tuple(
            elems
                .iter()
                .map(|e| {
                    resolve_type_expr_with_subst_opts(
                        &e.value, tag_types, subst, tag_params, tag_decls,
                    )
                })
                .collect(),
        ),
    }
}

fn resolve_in_range_bounds(bounds: &InRangeBounds, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    match bounds {
        InRangeBounds::Literal(min, max) => Ty::bounded_int(*min, *max),
        InRangeBounds::Tag(name) => {
            if let Some(ty) = tag_types.get(name) {
                if ty.is_bounded_int() {
                    return ty.clone();
                }
                if let Some(b) = ty.scalar_bounds_from_range_value() {
                    return Ty::Int {
                        width: ty_int_width(ty).unwrap_or(64),
                        signed: b.min < 0,
                        value: None,
                        min: Some(b.min),
                        max: Some(b.max),
                    };
                }
            }
            Ty::Opaque(*name)
        }
    }
}

fn ty_int_width(ty: &Ty) -> Option<u8> {
    match ty {
        Ty::Record { fields, .. } => {
            fields
                .iter()
                .find(|(n, _)| n.as_str() == "start")
                .and_then(|(_, t)| match t.as_ref() {
                    Ty::Int { width, .. } => Some(*width),
                    _ => None,
                })
        }
        Ty::Int { width, .. } => Some(*width),
        _ => None,
    }
}

/// Flattens a qualified path into a single symbol name for codegen (e.g. `io.print`).
pub fn mangled_fn_call_name(call: &FnCall) -> Intern<String> {
    if call.path.segments.is_empty() {
        call.path.root
    } else {
        let mut joined = call.path.root.as_str().to_string();
        for seg in &call.path.segments {
            joined.push('.');
            joined.push_str(seg.as_str());
        }
        Intern::<String>::new(joined)
    }
}

pub fn resolve_name_from_files(
    name: Intern<String>,
    files: &[ast::FileAst],
    _recursion_depth: usize,
) -> Ty {
    let mut raw: HashMap<Intern<String>, &DeclareValue> = HashMap::new();
    for ast in files {
        for (k, v) in ast.tags.iter() {
            raw.insert(*k, &v.value);
        }
    }
    // Look up the name in the raw declare map
    if let Some(dv) = raw.get(&name) {
        match dv {
            DeclareValue::Alias(spanned) => {
                let empty: HashMap<Intern<String>, Ty> = HashMap::new();
                return resolve_type_expr_with_subst(&spanned.value, &empty, &empty, None);
            }
            DeclareValue::Interface(members) => {
                let fields: Vec<(Intern<String>, Box<Ty>)> = members
                    .iter()
                    .map(|m| {
                        let ty = m
                            .return_ty
                            .as_ref()
                            .map(|rt| {
                                resolve_type_expr_with_subst(
                                    &rt.value,
                                    &HashMap::new(),
                                    &HashMap::new(),
                                    None,
                                )
                            })
                            .unwrap_or(Ty::Unit);
                        (m.name, Box::new(ty))
                    })
                    .collect();
                return Ty::Record { name, fields };
            }
            DeclareValue::Union { .. } => {
                return Ty::Union {
                    name,
                    variants: Vec::new(),
                    literal_values: None,
                };
            }
            _ => {}
        }
    }
    Ty::Opaque(name)
}

fn build_subst_for_generic(
    use_site_params: &[(Intern<String>, ParameterKind)],
    tag_types: &HashMap<Intern<String>, Ty>,
    outer_subst: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
    tag_name: &Intern<String>,
    tag_decls: Option<&ast::TagMap>,
) -> HashMap<Intern<String>, Ty> {
    let mut out = HashMap::new();

    // Process use-site args by positional correspondence with declaration params.
    if let Some(decl_params) = tag_params.and_then(|tp| tp.get(tag_name)) {
        let decl_entries: Vec<_> = decl_params.iter().collect();

        for (i, (name, kind)) in use_site_params.iter().enumerate() {
            let resolved = match kind {
                ParameterKind::Tagged(sp) if sp.value.is_type_surface() => {
                    resolve_type_expr_with_subst_opts(
                        &sp.value,
                        tag_types,
                        outer_subst,
                        tag_params,
                        tag_decls,
                    )
                }
                ParameterKind::Generic => Ty::Opaque(*name),
                _ => continue,
            };
            // Map the resolved type to the declaration param at this position.
            if let Some((decl_name, _)) = decl_entries.get(i) {
                out.insert(**decl_name, resolved);
            }
        }

        // Fill defaults for any remaining declaration params that weren't provided.
        for (decl_name, decl_kind) in decl_entries.iter().skip(use_site_params.len()) {
            if let ParameterKind::Default(expr) = decl_kind
                && let Some(te) = expr.value.as_type_expr()
                && te.is_type_surface()
            {
                let default_ty = resolve_type_expr_with_subst_opts(
                    &te,
                    tag_types,
                    outer_subst,
                    tag_params,
                    tag_decls,
                );
                out.insert(**decl_name, default_ty);
            }
        }
    } else {
        // No declaration params available — fall back to old behavior.
        for (name, kind) in use_site_params {
            let resolved = match kind {
                ParameterKind::Tagged(sp) if sp.value.is_type_surface() => {
                    resolve_type_expr_with_subst_opts(
                        &sp.value,
                        tag_types,
                        outer_subst,
                        tag_params,
                        tag_decls,
                    )
                }
                ParameterKind::Generic => Ty::Opaque(*name),
                _ => continue,
            };
            out.insert(*name, resolved);
        }
    }

    out
}
