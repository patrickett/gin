//! Raw type resolution — translating AST type declarations into resolved [`Ty`] values.
//!
//! These functions operate on the raw [`DeclareValue`] tree without a resolved
//! `HashMap`. Resolution unfolds aliases, computes union variants and const-unions,
//! and produces the canonical [`Ty`] that other passes consume.

use crate::analysis::const_value::ConstValue;
use crate::ty::{Ty, str_record_ty};
use crate::{DeclareValue, FnCall, Literal, ParameterKind, TypeExpr, type_surface_mangle_name};
use i256::I256;
use internment::Intern;
use std::collections::HashMap;

/// Check whether a [`TypeExpr`] is a type surface.
pub fn is_type_surface(e: &TypeExpr) -> bool {
    matches!(
        e,
        TypeExpr::Nominal(..) | TypeExpr::Qualified(_) | TypeExpr::Generic { .. }
    )
}

/// Collect every type-parameter name introduced by a method's receiver type
/// surface, e.g. `x` in `Range[x].new(...)`. The returned substitution maps
/// each name to `Ty::Opaque(name)` (the identity binding used while
/// typechecking the method itself).
///
/// Returns an empty map for non-`Generic` receivers (`Bool.to_string`
/// introduces no type variables).
pub fn typevars_from_receiver(recv: &TypeExpr) -> HashMap<Intern<String>, Ty> {
    let mut out = HashMap::new();
    if let TypeExpr::Generic { params, .. } = recv {
        for (name, kind) in params {
            if matches!(kind, ParameterKind::Generic) {
                out.insert(*name, Ty::Opaque(*name));
            }
        }
    }
    out
}

/// Resolve a type-surface [`TypeExpr`] to a [`Ty`] using a pre-built `tag_types` map.
pub fn resolve_type_expr_from_map(e: &TypeExpr, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    let empty: HashMap<Intern<String>, Ty> = HashMap::new();
    resolve_type_expr_with_subst(e, tag_types, &empty)
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
pub fn resolve_type_expr_with_subst(
    e: &TypeExpr,
    tag_types: &HashMap<Intern<String>, Ty>,
    subst: &HashMap<Intern<String>, Ty>,
) -> Ty {
    match e {
        TypeExpr::Nominal(name, _) => {
            if let Some(t) = subst.get(name) {
                return t.clone();
            }
            tag_types.get(name).cloned().unwrap_or(Ty::i64())
        }
        TypeExpr::Generic { name, params, .. } => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        ParameterKind::Tagged(sp) => sp
                            .value
                            .as_type_expr()
                            .map(|te| resolve_type_expr_with_subst(&te, tag_types, subst)),
                        _ => None,
                    })
                    .unwrap_or(Ty::Opaque(*name));
                if name.as_str() == "Ptr" {
                    Ty::Ptr {
                        inner: Box::new(inner),
                    }
                } else {
                    Ty::Ref {
                        inner: Box::new(inner),
                    }
                }
            }
            _ => {
                let base = tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name));
                let local_subst = build_subst_for_generic(params, tag_types, subst);
                if local_subst.is_empty() {
                    base
                } else {
                    substitute_in_ty(&base, &local_subst)
                }
            }
        },
        TypeExpr::Qualified(path) => tag_types
            .get(&path.root)
            .cloned()
            .unwrap_or(Ty::Opaque(path.root)),
        TypeExpr::Literal(..) => Ty::Opaque(Intern::new(String::new())),
    }
}

/// Walk a `Ty` and replace every `Ty::Opaque(name)` with `subst[name]` when
/// present. Recurses through Records, Unions, Tuples, Arrays, Ptr, and Ref.
pub fn substitute_in_ty(ty: &Ty, subst: &HashMap<Intern<String>, Ty>) -> Ty {
    match ty {
        Ty::Opaque(name) => subst.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        Ty::Record { name, fields } => Ty::Record {
            name: *name,
            fields: fields
                .iter()
                .map(|(n, t)| (*n, Box::new(substitute_in_ty(t, subst))))
                .collect(),
        },
        Ty::Union { name, variants } => Ty::Union {
            name: *name,
            variants: variants
                .iter()
                .map(|(vn, fields)| {
                    let new_fields = fields
                        .iter()
                        .map(|(n, t)| (*n, Box::new(substitute_in_ty(t, subst))))
                        .collect();
                    (*vn, new_fields)
                })
                .collect(),
        },
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|t| substitute_in_ty(t, subst)).collect()),
        Ty::Array { elem, size } => Ty::Array {
            elem: Box::new(substitute_in_ty(elem, subst)),
            size: *size,
        },
        Ty::Ptr { inner } => Ty::Ptr {
            inner: Box::new(substitute_in_ty(inner, subst)),
        },
        Ty::Ref { inner } => Ty::Ref {
            inner: Box::new(substitute_in_ty(inner, subst)),
        },
        _ => ty.clone(),
    }
}

/// Build the symbol name for a callee: `foo` or `io.print` (matches codegen).
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

/// Resolve a declared tag name to its full `Ty` by looking through the raw
/// `DeclareValue` declarations across one or more `FileAst`s.
pub fn resolve_name_from_files(
    name: Intern<String>,
    files: &[crate::FileAst],
    recursion_depth: usize,
) -> Ty {
    let mut raw: HashMap<Intern<String>, &DeclareValue> = HashMap::new();
    for ast in files {
        for (k, v) in ast.tags.iter() {
            raw.insert(*k, v.value());
        }
    }
    resolve_name(name, &raw, recursion_depth)
}

// ── Internal helpers ───────────────────────────────────────────────────────

fn build_subst_for_generic(
    params: &[(Intern<String>, ParameterKind)],
    tag_types: &HashMap<Intern<String>, Ty>,
    outer_subst: &HashMap<Intern<String>, Ty>,
) -> HashMap<Intern<String>, Ty> {
    let mut out = HashMap::new();
    for (name, kind) in params {
        let resolved = match kind {
            ParameterKind::Tagged(sp)
                if sp
                    .value
                    .as_type_expr()
                    .is_some_and(|te| is_type_surface(&te)) =>
            {
                resolve_type_expr_with_subst(
                    &sp.value.as_type_expr().unwrap(),
                    tag_types,
                    outer_subst,
                )
            }
            ParameterKind::Generic => Ty::Opaque(*name),
            _ => continue,
        };
        out.insert(*name, resolved);
    }
    out
}

fn range_bit_width(min: I256, max: I256) -> u8 {
    let range = max - min;
    if range <= I256::from_i128(u8::MAX as i128 + 1) {
        8
    } else if range <= I256::from_i128(u16::MAX as i128 + 1) {
        16
    } else if range <= I256::from_i128(u32::MAX as i128 + 1) {
        32
    } else if range <= I256::from_i128(u64::MAX as i128 + 1) {
        64
    } else {
        128
    }
}

fn resolve_type_expr_ref(
    e: &TypeExpr,
    raw: &HashMap<Intern<String>, &DeclareValue>,
    recursion_depth: usize,
) -> Ty {
    match e {
        TypeExpr::Nominal(name, _) => resolve_name(*name, raw, recursion_depth),
        TypeExpr::Generic { name, params, .. } => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        ParameterKind::Tagged(sp) => sp
                            .value
                            .as_type_expr()
                            .map(|te| resolve_type_expr_ref(&te, raw, recursion_depth + 1)),
                        _ => None,
                    })
                    .unwrap_or(Ty::Opaque(*name));
                if name.as_str() == "Ptr" {
                    Ty::Ptr {
                        inner: Box::new(inner),
                    }
                } else {
                    Ty::Ref {
                        inner: Box::new(inner),
                    }
                }
            }
            _ => resolve_name(*name, raw, recursion_depth + 1),
        },
        TypeExpr::Qualified(path) => resolve_name(path.root, raw, recursion_depth),
        TypeExpr::Literal(..) => Ty::Opaque(Intern::new(String::new())),
    }
}

fn resolve_name(
    name: Intern<String>,
    raw: &HashMap<Intern<String>, &DeclareValue>,
    recursion_depth: usize,
) -> Ty {
    if recursion_depth > 16 {
        return Ty::Opaque(name);
    }
    match raw.get(&name) {
        Some(DeclareValue::Alias(sp)) => {
            if is_type_surface(&sp.value) {
                resolve_type_expr_ref(&sp.value, raw, recursion_depth + 1)
            } else {
                Ty::Opaque(name)
            }
        }
        Some(DeclareValue::Range(start, end)) => Ty::Int {
            width: range_bit_width(*start, *end),
            signed: start.is_negative(),
            value: None,
        },
        Some(DeclareValue::InRange(start, end)) => Ty::Int {
            width: range_bit_width(*start, *end),
            signed: start.is_negative(),
            value: None,
        },
        Some(DeclareValue::Record(params)) => {
            let fields = params
                .iter()
                .map(|(field_name, kind)| {
                    let field_ty = match kind {
                        ParameterKind::Tagged(sp) => {
                            if let Some(te) = sp.value.as_type_expr()
                                && is_type_surface(&te)
                            {
                                resolve_type_expr_ref(&te, raw, recursion_depth + 1)
                            } else {
                                Ty::Opaque(*field_name)
                            }
                        }
                        ParameterKind::Generic => Ty::Opaque(*field_name),
                        ParameterKind::Default(_) => Ty::Int {
                            width: 64,
                            signed: true,
                            value: None,
                        },
                    };
                    (*field_name, Box::new(field_ty))
                })
                .collect();
            Ty::Record { name, fields }
        }
        Some(DeclareValue::Union { variants }) => {
            let mut lit_values = Vec::new();
            let mut lit_base: Option<Ty> = None;
            let mut tag_variants = Vec::new();

            for v in variants {
                let shape = &v.shape().value;
                // Check if the shape is a nominal that looks like a literal
                // (e.g. anonymous tags like True/False are represented as TypeExpr::Nominal)
                if let Some(cv) = const_value_from_type_expr(shape) {
                    let base_ty = base_ty_for_const_value(&cv);
                    lit_base.get_or_insert_with(|| base_ty.clone());
                    lit_values.push(cv);
                } else if is_type_surface(shape) {
                    let variant_name = Intern::<String>::from_ref(type_surface_mangle_name(shape));
                    let fields = match shape {
                        TypeExpr::Generic { params, .. } if !params.is_empty() => params
                            .iter()
                            .filter_map(|(field_name, kind)| match kind {
                                ParameterKind::Tagged(sp) => {
                                    sp.value.as_type_expr().filter(is_type_surface).map(|te| {
                                        (
                                            *field_name,
                                            Box::new(resolve_type_expr_ref(
                                                &te,
                                                raw,
                                                recursion_depth + 1,
                                            )),
                                        )
                                    })
                                }
                                ParameterKind::Generic => {
                                    Some((*field_name, Box::new(Ty::Opaque(*field_name))))
                                }
                                _ => None,
                            })
                            .collect(),
                        _ => vec![],
                    };
                    tag_variants.push((variant_name, fields));
                }
            }

            if !lit_values.is_empty() {
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
        _ => Ty::Opaque(name),
    }
}

fn const_value_from_type_expr(e: &TypeExpr) -> Option<ConstValue> {
    match e {
        TypeExpr::Literal(lit, _) => const_value_from_literal(lit),
        _ => None,
    }
}

fn const_value_from_literal(lit: &Literal) -> Option<ConstValue> {
    match lit {
        Literal::String(s) => Some(ConstValue::String(s.clone())),
        Literal::Int(n) => Some(ConstValue::Int(*n as i128)),
        Literal::Float(f) => Some(ConstValue::Float(*f)),
        Literal::Number(n) => Some(ConstValue::Int(*n as i128)),
    }
}

fn base_ty_for_const_value(cv: &ConstValue) -> Ty {
    match cv {
        ConstValue::String(_) => str_record_ty(),
        ConstValue::Int(_) => Ty::Int {
            width: 64,
            signed: true,
            value: None,
        },
        ConstValue::Float(_) => Ty::Float { value: None },
        ConstValue::Tag { .. } => Ty::Opaque(Intern::<String>::from_ref("Tag")),
        ConstValue::Record { .. } => Ty::Opaque(Intern::<String>::from_ref("Record")),
        ConstValue::List(_) => Ty::Opaque(Intern::<String>::from_ref("List")),
    }
}
