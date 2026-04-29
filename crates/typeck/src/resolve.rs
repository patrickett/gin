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

pub(crate) fn resolve_type_expr_from_map(e: &Expr, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    match e {
        Expr::TypeNominal(name, _) => tag_types.get(name).cloned().unwrap_or(Ty::Int {
            width: 64,
            signed: true,
            value: None,
        }),
        Expr::TypeGeneric { name, params, .. } => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        ParameterKind::Tagged(sp) => {
                            Some(resolve_type_expr_from_map(&sp.0, tag_types))
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
            _ => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        },
        Expr::TypeQualified(path) => tag_types
            .get(&path.root)
            .cloned()
            .unwrap_or(Ty::Opaque(path.root)),
        _ => Ty::Opaque(Intern::<String>::from_ref("?")),
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
        Expr::TypeQualified(path) => {
            resolve_name(path.root, raw, recursion_depth)
        }
        _ => Ty::Opaque(Intern::<String>::from_ref("?")),
    }
}

pub(crate) fn resolve_name_from_files(name: Intern<String>, files: &[ast::FileAst], recursion_depth: usize) -> Ty {
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
