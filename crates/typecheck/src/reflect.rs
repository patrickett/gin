//! Compile-time reflection: map resolved [`Ty`] to the stdlib `Type` ADT as [`ConstValue`].

use internment::Intern;

use crate::ty::Ty;
use ast::expr::{Expr, Literal, TagCall, Typed};
use ast::span::SpanId;
use ast::{ConstValue, HashFloat};
use std::collections::HashSet;

/// Build a `ConstValue` describing `ty` for the synthesized `Reflectable.shape` field.
pub fn reflect_ty_to_const_value(ty: &Ty) -> ConstValue {
    let mut seen: HashSet<Intern<String>> = HashSet::new();
    reflect_ty_to_const_value_inner(ty, &mut seen)
}

fn reflect_ty_to_const_value_inner(ty: &Ty, seen: &mut HashSet<Intern<String>>) -> ConstValue {
    match ty {
        Ty::Int { width, signed, .. } => tag(
            "Primitive",
            vec![
                ConstValue::Int(i128::from(*width)),
                ConstValue::Tag {
                    name: Intern::from_ref(if *signed { "True" } else { "False" }),
                    qual_path: None,
                    args: vec![].into(),
                },
            ],
        ),
        Ty::Float { .. } => tag(
            "Primitive",
            vec![
                ConstValue::Int(64),
                ConstValue::Tag {
                    name: Intern::from_ref("True"),
                    qual_path: None,
                    args: vec![].into(),
                },
            ],
        ),
        Ty::Unit => tag("Tuple", vec![ConstValue::List(vec![].into())]),
        Ty::Record { name, fields } => {
            // Break nominal cycles like `NamedTy.ty: Type` (and `Type` includes `NamedTy`).
            if !seen.insert(*name) {
                return tag(
                    "Opaque",
                    vec![ConstValue::String(name.as_str().to_string())],
                );
            }
            let named: Vec<ConstValue> = fields
                .iter()
                .map(|(fname, fty)| {
                    record_named(fname.as_str(), reflect_ty_to_const_value_inner(fty, seen))
                })
                .collect();
            seen.remove(name);
            tag(
                "Record",
                vec![
                    ConstValue::String(name.as_str().to_string()),
                    ConstValue::List(named.into()),
                ],
            )
        }
        Ty::Union {
            name,
            variants,
            literal_values: Some(values),
            ..
        } if !values.is_empty() => {
            if !seen.insert(*name) {
                return tag(
                    "Opaque",
                    vec![ConstValue::String(name.as_str().to_string())],
                );
            }
            let variant_shapes: Vec<ConstValue> = values
                .iter()
                .map(|cv| {
                    let vname = cv.to_hover_string();
                    record_variant(&vname, vec![record_named("value", cv.clone())])
                })
                .collect();
            seen.remove(name);
            tag(
                "Union",
                vec![
                    ConstValue::String(name.as_str().to_string()),
                    ConstValue::List(variant_shapes.into()),
                ],
            )
        }
        Ty::Union { name, variants, .. } => {
            if !seen.insert(*name) {
                return tag(
                    "Opaque",
                    vec![ConstValue::String(name.as_str().to_string())],
                );
            }
            let variant_shapes: Vec<ConstValue> = variants
                .iter()
                .map(|(vname, vfields)| {
                    let fields: Vec<ConstValue> = vfields
                        .iter()
                        .map(|(fname, fty)| {
                            record_named(fname.as_str(), reflect_ty_to_const_value_inner(fty, seen))
                        })
                        .collect();
                    record_variant(vname.as_str(), fields)
                })
                .collect();
            seen.remove(name);
            tag(
                "Union",
                vec![
                    ConstValue::String(name.as_str().to_string()),
                    ConstValue::List(variant_shapes.into()),
                ],
            )
        }
        Ty::Tuple(elems) => {
            let items: Vec<ConstValue> = elems
                .iter()
                .map(|t| reflect_ty_to_const_value_inner(t, seen))
                .collect();
            tag("Tuple", vec![ConstValue::List(items.into())])
        }
        Ty::Ptr { inner } => tag("Ptr", vec![reflect_ty_to_const_value_inner(inner, seen)]),
        Ty::Ref { inner, mutable } => tag(
            "Ref",
            vec![
                reflect_ty_to_const_value_inner(inner, seen),
                ConstValue::Tag {
                    name: Intern::from_ref(if *mutable { "True" } else { "False" }),
                    qual_path: None,
                    args: vec![].into(),
                },
            ],
        ),
        Ty::Array { elem, size } => {
            let size_val = match size {
                ast::ConstExpr::Value(v) => v.clone(),
                _ => ConstValue::Int(0),
            };
            tag(
                "Array",
                vec![reflect_ty_to_const_value_inner(elem, seen), size_val],
            )
        }
        Ty::Opaque(name) => tag(
            "Opaque",
            vec![ConstValue::String(name.as_str().to_string())],
        ),
        Ty::Literal(cv) => record_named("Literal", cv.clone()),
    }
}

fn tag(name: &str, args: Vec<ConstValue>) -> ConstValue {
    ConstValue::Tag {
        name: Intern::new(name.to_string()),
        qual_path: None,
        args: args.into(),
    }
}

fn record_named(name: &str, ty: ConstValue) -> ConstValue {
    ConstValue::Record {
        fields: vec![
            (
                Intern::new("name".to_string()),
                ConstValue::String(name.to_string()),
            ),
            (Intern::new("ty".to_string()), ty),
        ]
        .into(),
    }
}

fn record_variant(name: &str, fields: Vec<ConstValue>) -> ConstValue {
    ConstValue::Record {
        fields: vec![
            (
                Intern::new("name".to_string()),
                ConstValue::String(name.to_string()),
            ),
            (
                Intern::new("fields".to_string()),
                ConstValue::List(fields.into()),
            ),
        ]
        .into(),
    }
}

/// Store a compile-time [`ConstValue`] on a provided-trait field without building a full
/// mirrored [`Expr`] tree (that duplicates the whole shape and can use gigabytes for `Type`).
pub fn const_value_for_provided_trait_field(cv: ConstValue, span_id: SpanId) -> Typed<Expr> {
    Typed {
        value: Expr::Lit(Literal::Number(0)),
        ty: ast::ty_state::TyState::Infer,
        const_value: Some(cv),
        span_id,
    }
}

/// Turn a [`ConstValue`] into a [`Typed<Expr>`] for storage in [`ProvidedTrait`] fields.
pub fn const_value_to_typed_expr(cv: &ConstValue, span_id: SpanId) -> Typed<Expr> {
    Typed {
        value: const_value_to_expr(cv),
        ty: ast::ty_state::TyState::Infer,
        const_value: Some(cv.clone()),
        span_id,
    }
}

pub fn const_value_to_expr(cv: &ConstValue) -> Expr {
    match cv {
        ConstValue::String(s) => Expr::Lit(Literal::String(s.clone())),
        ConstValue::Int(n) => {
            let n = u128::try_from(*n).unwrap_or(0);
            Expr::Lit(Literal::Int(n))
        }
        ConstValue::Float(HashFloat(f)) => Expr::Lit(Literal::Float(HashFloat(*f))),
        ConstValue::Tag { name, args, .. } if args.is_empty() => Expr::AnonymousTag(*name),
        ConstValue::Tag {
            name,
            qual_path,
            args,
        } => Expr::TagCall(TagCall {
            name: *name,
            qual_path: qual_path.as_ref().map(|s| {
                let parts: Vec<_> = s.split('.').map(Intern::from_ref).collect();
                let root = parts[0];
                let segments = parts.get(1..).unwrap_or(&[]).to_vec();
                ast::Spanned::new(ast::ModPath::new(root, segments), SpanId::INVALID)
            }),
            args: args
                .iter()
                .map(|a| const_value_to_typed_expr(a, SpanId::INVALID))
                .collect(),
        }),
        ConstValue::Record { fields } => {
            let args: Vec<Typed<Expr>> = fields
                .iter()
                .map(|(n, v)| {
                    let mut b = ast::Bind::new(*n, SpanId::INVALID, ast::BindValue::Unassigned);
                    b.value = ast::BindValue::Expr(Box::new(const_value_to_typed_expr(
                        v,
                        SpanId::INVALID,
                    )));
                    Typed {
                        value: Expr::Bind(Box::new(b)),
                        ty: ast::ty_state::TyState::Infer,
                        const_value: Some(v.clone()),
                        span_id: SpanId::INVALID,
                    }
                })
                .collect();
            Expr::TagCall(TagCall {
                name: Intern::from_ref("Record"),
                qual_path: None,
                args,
            })
        }
        ConstValue::List(items) => Expr::List(
            items
                .iter()
                .map(|i| const_value_to_typed_expr(i, SpanId::INVALID))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile_time_trait::synthesize_reflectable_trait;
    use crate::ty::Ty;
    use internment::Intern;

    fn intern(s: &str) -> Intern<String> {
        Intern::new(s.to_owned())
    }

    #[test]
    fn reflect_type_union_cycle_stays_bounded() {
        let named_ty = Ty::Record {
            name: intern("NamedTy"),
            fields: vec![
                (intern("name"), Box::new(Ty::Opaque(intern("String")))),
                (intern("ty"), Box::new(Ty::Opaque(intern("Type")))),
            ],
        };
        let list_named = Ty::Record {
            name: intern("List"),
            fields: vec![(intern("pointer"), Box::new(Ty::Opaque(intern("Pointer"))))],
        };
        let type_union = Ty::Union {
            name: intern("Type"),
            variants: vec![(
                intern("Record"),
                vec![
                    (intern("name"), Box::new(Ty::Opaque(intern("String")))),
                    (intern("fields"), Box::new(list_named)),
                ],
            )],
            literal_values: None,
        };
        let _ = reflect_ty_to_const_value(&type_union);
        let _ = reflect_ty_to_const_value(&named_ty);
    }

    #[test]
    fn synthesize_reflectable_trait_shape_is_const_value_only() {
        let ty = Ty::Union {
            name: Intern::from_ref("Bool"),
            variants: vec![
                (Intern::from_ref("True"), vec![]),
                (Intern::from_ref("False"), vec![]),
            ],
            literal_values: None,
        };
        let pt = synthesize_reflectable_trait(&ty);
        let shape = &pt.fields[0].1;
        assert!(shape.const_value.is_some());
        assert!(matches!(shape.value, Expr::Lit(Literal::Number(0))));
    }
}
