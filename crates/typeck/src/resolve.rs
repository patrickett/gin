//! Name resolution — translating AST type declarations into resolved [`Ty`] values.

use ast::{DeclareValue, Expr, FnCall, ParameterKind, type_surface_mangle_name};
use i256::I256;
use internment::Intern;
use std::collections::HashMap;

use crate::ty::Ty;

/// Symbol name for a callee: `foo` or `io.print` (matches codegen).
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

pub(crate) fn is_type_surface(e: &Expr) -> bool {
    matches!(
        e,
        Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. }
    )
}

/// Collect every type-parameter name introduced by a method's receiver type
/// surface, e.g. `x` in `Range(x).new(...)`. The returned substitution maps
/// each name to `Ty::Opaque(name)` (the identity binding used while
/// typechecking the method itself).
///
/// Returns an empty map for non-`TypeGeneric` receivers (`Bool.to_string`
/// introduces no type variables).
pub(crate) fn typevars_from_receiver(recv: &Expr) -> HashMap<Intern<String>, Ty> {
    let mut out = HashMap::new();
    if let Expr::TypeGeneric { params, .. } = recv {
        for (name, kind) in params {
            // Only bare `Generic` parameters introduce a type variable. A
            // `Tagged` arg means the receiver is being instantiated with a
            // concrete type at this site (rare for a definition).
            if matches!(kind, ParameterKind::Generic) {
                out.insert(*name, Ty::Opaque(*name));
            }
        }
    }
    out
}

pub(crate) fn resolve_type_expr_from_map(e: &Expr, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    let empty: HashMap<Intern<String>, Ty> = HashMap::new();
    resolve_type_expr_with_subst(e, tag_types, &empty)
}

/// Resolve a type-surface expression to a [`Ty`], substituting any type-variable
/// names found in `subst`.
///
/// `subst` maps method-scoped type variable names (e.g. `x` in
/// `Range(x).new(start x, end x) Range(x)`) to the `Ty` they currently stand for.
/// During typechecking of the method body itself, the substitution is the identity
/// (`x -> Ty::Opaque(x)`) so the same opaque tag flows through params, body, and
/// return type. At call sites, `subst` can bind `x` to a concrete type (e.g.
/// `Ty::Int`) to instantiate the signature.
///
/// Substitution applies in two places:
/// - `Expr::TypeNominal(name, _)` where `name` is in `subst` (bare type-var use,
///   e.g. `start x`).
/// - The fields of a `Ty::Record`/`Ty::Union` looked up via
///   `Expr::TypeGeneric { name, params, .. }`. Each `Ty::Opaque(p)` field is
///   replaced with `subst[p]` if the param `p` appears in the generic call's
///   arguments. This implements `Range(x)` → `Record { start: Opaque(x), end: Opaque(x) }`
///   when called with the identity subst, and `Range(Int)` → `Record { start: Int, end: Int }`
///   when called with `{ x -> Int }`.
pub(crate) fn resolve_type_expr_with_subst(
    e: &Expr,
    tag_types: &HashMap<Intern<String>, Ty>,
    subst: &HashMap<Intern<String>, Ty>,
) -> Ty {
    match e {
        Expr::TypeNominal(name, _) => {
            if let Some(t) = subst.get(name) {
                return t.clone();
            }
            tag_types.get(name).cloned().unwrap_or(Ty::Int {
                width: 64,
                signed: true,
                value: None,
            })
        }
        Expr::TypeGeneric { name, params, .. } => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        ParameterKind::Tagged(sp) => {
                            Some(resolve_type_expr_with_subst(&sp.0, tag_types, subst))
                        }
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
        Expr::TypeQualified(path) => tag_types
            .get(&path.root)
            .cloned()
            .unwrap_or(Ty::Opaque(path.root)),
        _ => Ty::Opaque(Intern::<String>::from_ref("?")),
    }
}

/// Build a substitution map from the generic-call params at a use site.
///
/// Maps each declared type-parameter name (the key in `params`, e.g. `x` in
/// `Range(x)`) to the `Ty` resolved from the corresponding `Tagged` argument
/// (or to `Ty::Opaque(name)` for bare `Generic` args, which is the identity
/// case used while typechecking the method itself).
fn build_subst_for_generic(
    params: &[(Intern<String>, ParameterKind)],
    tag_types: &HashMap<Intern<String>, Ty>,
    outer_subst: &HashMap<Intern<String>, Ty>,
) -> HashMap<Intern<String>, Ty> {
    let mut out = HashMap::new();
    for (name, kind) in params {
        let resolved = match kind {
            ParameterKind::Tagged(sp) if is_type_surface(&sp.0) => {
                resolve_type_expr_with_subst(&sp.0, tag_types, outer_subst)
            }
            ParameterKind::Generic => Ty::Opaque(*name),
            _ => continue,
        };
        out.insert(*name, resolved);
    }
    out
}

/// Walk a `Ty` and replace every `Ty::Opaque(name)` with `subst[name]` when
/// present. Recurses through Records, Unions, Tuples, Arrays, Ptr, and Ref.
pub(crate) fn substitute_in_ty(ty: &Ty, subst: &HashMap<Intern<String>, Ty>) -> Ty {
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
    e: &Expr,
    raw: &HashMap<Intern<String>, &DeclareValue>,
    recursion_depth: usize,
) -> Ty {
    match e {
        Expr::TypeNominal(name, _) => resolve_name(*name, raw, recursion_depth),
        Expr::TypeGeneric { name, params, .. } => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        ParameterKind::Tagged(sp) => {
                            Some(resolve_type_expr_ref(&sp.0, raw, recursion_depth + 1))
                        }
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
        Expr::TypeQualified(path) => resolve_name(path.root, raw, recursion_depth),
        _ => Ty::Opaque(Intern::<String>::from_ref("?")),
    }
}

pub(crate) fn resolve_name_from_files(
    name: Intern<String>,
    files: &[ast::FileAst],
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
            if is_type_surface(&sp.0) {
                resolve_type_expr_ref(&sp.0, raw, recursion_depth + 1)
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
                            if is_type_surface(&sp.0) {
                                resolve_type_expr_ref(&sp.0, raw, recursion_depth + 1)
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
            let resolved = variants
                .iter()
                .filter_map(|v| {
                    let shape = &v.shape().0;
                    if !is_type_surface(shape) {
                        return None;
                    }
                    let variant_name = Intern::<String>::from_ref(type_surface_mangle_name(shape));
                    let fields = match shape {
                        Expr::TypeGeneric { params, .. } if !params.is_empty() => params
                            .iter()
                            .filter_map(|(field_name, kind)| match kind {
                                ParameterKind::Tagged(sp) => is_type_surface(&sp.0).then_some((
                                    *field_name,
                                    Box::new(resolve_type_expr_ref(
                                        &sp.0,
                                        raw,
                                        recursion_depth + 1,
                                    )),
                                )),
                                ParameterKind::Generic => {
                                    Some((*field_name, Box::new(Ty::Opaque(*field_name))))
                                }
                                _ => None,
                            })
                            .collect(),
                        _ => vec![],
                    };
                    Some((variant_name, fields))
                })
                .collect();
            Ty::Union {
                name,
                variants: resolved,
            }
        }
        _ => Ty::Opaque(name),
    }
}
