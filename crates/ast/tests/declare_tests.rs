use ast::declare::{
    Declare, DeclareValue, HasFunction, HasFunctionKind, HasMember, HasMemberBody,
    HasMemberQualifier, HasProperty,
};
use ast::parameter::{Parameter, ParameterKind};
use ast::span::{SpanId, Spanned};
use ast::{BindValue, Expr, Literal, Pattern, TypeExpr, Typed, Variant};
use ast_format::declare::DeclareFormatExt;
use internment::Intern;

fn intern(value: &str) -> Intern<String> {
    Intern::new(value.to_string())
}

fn nominal(name: &str) -> TypeExpr {
    TypeExpr::Nominal(intern(name), SpanId::INVALID)
}

fn spanned_expr(value: Expr) -> Spanned<Expr> {
    Spanned {
        value,
        span_id: SpanId::INVALID,
    }
}

fn expr_from_type(value: TypeExpr) -> Expr {
    match value {
        TypeExpr::Nominal(name, _) => Expr::AnonymousTag(name),
        TypeExpr::Generic { name, params, .. } => Expr::TagCall(ast::TagCall {
            name,
            qual_path: None,
            args: params
                .into_iter()
                .map(|(_, kind)| match kind {
                    ParameterKind::Tagged(sp)
                    | ParameterKind::ValueParam { ty: sp }
                    | ParameterKind::Inferred { ty: sp } => Typed::infer(sp.value, SpanId::INVALID),
                    ParameterKind::Generic => Typed::infer(
                        Expr::AnonymousTag(Intern::new(String::new())),
                        SpanId::INVALID,
                    ),
                    ParameterKind::Default(expr) => *expr,
                })
                .collect(),
        }),
        TypeExpr::Literal(lit, _) => Expr::Lit(lit),
        _ => Expr::Lit(Literal::Number(0)),
    }
}

fn tagged(value: TypeExpr) -> ParameterKind {
    ParameterKind::Tagged(Box::new(Spanned {
        value: expr_from_type(value),
        span_id: SpanId::INVALID,
    }))
}

fn generic(name: &str, params: Vec<(&str, TypeExpr)>) -> TypeExpr {
    TypeExpr::Generic {
        name: intern(name),
        params: params
            .into_iter()
            .map(|(name, ty)| (intern(name), tagged(ty)))
            .collect(),
        param_spans: Vec::new(),
        span: SpanId::INVALID,
    }
}

fn variant(shape: TypeExpr) -> Variant {
    Variant::External {
        shape: Box::new(Spanned {
            value: Pattern::from(shape),
            span_id: SpanId::INVALID,
        }),
        result_ty: None,
    }
}

#[test]
fn declaration_surface_keeps_simple_nominal_union_compact() {
    let declare = Declare::new(
        intern("Bool"),
        SpanId::INVALID,
        DeclareValue::Union {
            variants: vec![variant(nominal("True")), variant(nominal("False"))],
        },
    );

    assert_eq!(declare.surface_text(), "Bool is True or False");
}

#[test]
fn declaration_surface_empty_has_uses_has_keyword() {
    let declare = Declare::new(
        intern("AllocError"),
        SpanId::INVALID,
        DeclareValue::Has(Vec::new()),
    );

    assert_eq!(declare.surface_text(), "AllocError has");
}

#[test]
fn declaration_surface_simple_has_uses_inline() {
    let declare = Declare::new(
        intern("Bounded"),
        SpanId::INVALID,
        DeclareValue::Has(vec![
            HasMember::Property(HasProperty {
                qualifier: None,
                name: intern("min"),
                name_span: SpanId::INVALID,
                ty: None,
                body: None,
                doc_comment: None,
                refinement: None,
            }),
            HasMember::Property(HasProperty {
                qualifier: None,
                name: intern("max"),
                name_span: SpanId::INVALID,
                ty: None,
                body: None,
                doc_comment: None,
                refinement: None,
            }),
        ]),
    );

    assert_eq!(declare.surface_text(), "Bounded has min, max");
}

#[test]
fn declaration_surface_excludes_trait_implementation_members() {
    let declare = Declare::new(
        intern("Range"),
        SpanId::INVALID,
        DeclareValue::Has(vec![
            HasMember::Property(HasProperty {
                qualifier: None,
                name: intern("start"),
                name_span: SpanId::INVALID,
                ty: Some(Box::new(spanned_expr(expr_from_type(nominal("x"))))),
                body: None,
                doc_comment: None,
                refinement: None,
            }),
            HasMember::Property(HasProperty {
                qualifier: Some(HasMemberQualifier {
                    name: intern("Bounded"),
                    span: SpanId::INVALID,
                }),
                name: intern("min"),
                name_span: SpanId::INVALID,
                ty: None,
                body: Some(HasMemberBody::Overrideable(BindValue::Expr(Box::new(
                    Typed::infer(Expr::AnonymousTag(intern("start")), SpanId::INVALID),
                )))),
                doc_comment: None,
                refinement: None,
            }),
        ]),
    );

    assert_eq!(declare.surface_text(), "Range has start x");
}

#[test]
fn declaration_surface_has_members_uses_surface_types() {
    let declare = Declare::new(
        intern("Allocator"),
        SpanId::INVALID,
        DeclareValue::Has(vec![
            HasMember::Function(Box::new(HasFunction {
                qualifier: None,
                name: intern("reserve"),
                name_span: SpanId::INVALID,
                params: Box::new(
                    vec![(
                        intern("l"),
                        Parameter::new(SpanId::INVALID, tagged(nominal("Layout"))),
                    )]
                    .into_iter()
                    .collect(),
                ),
                conventions: Default::default(),
                param_groups: Default::default(),
                param_refinements: Default::default(),
                return_ty: Some(Box::new(spanned_expr(expr_from_type(generic(
                    "Slice",
                    vec![("Byte", nominal("Byte"))],
                ))))),
                error_ty: Some(Box::new(spanned_expr(expr_from_type(nominal(
                    "AllocError",
                ))))),
                body: None,
                doc_comment: None,
                refinement: None,
                kind: HasFunctionKind::Instance,
            })),
            HasMember::Function(Box::new(HasFunction {
                qualifier: None,
                name: intern("new"),
                name_span: SpanId::INVALID,
                params: Box::new(
                    vec![(
                        intern("size"),
                        Parameter::new(SpanId::INVALID, tagged(nominal("PointerSize"))),
                    )]
                    .into_iter()
                    .collect(),
                ),
                conventions: Default::default(),
                param_groups: Default::default(),
                param_refinements: Default::default(),
                return_ty: Some(Box::new(spanned_expr(expr_from_type(nominal("Self"))))),
                error_ty: None,
                body: Some(HasMemberBody::Overrideable(BindValue::Expr(Box::new(
                    Typed::infer(Expr::AnonymousTag(intern("Self")), SpanId::INVALID),
                )))),
                doc_comment: None,
                refinement: None,
                kind: HasFunctionKind::Associated,
            })),
            HasMember::Property(HasProperty {
                qualifier: None,
                name: intern("length"),
                name_span: SpanId::INVALID,
                ty: Some(Box::new(spanned_expr(expr_from_type(generic(
                    "Pointer",
                    vec![("Byte", nominal("Byte"))],
                ))))),
                body: None,
                doc_comment: None,
                refinement: None,
            }),
        ]),
    );

    assert_eq!(
        declare.surface_text(),
        "Allocator has\n    reserve(l Layout) Slice(Byte) or AllocError\n    length Pointer(Byte)"
    );
}

#[test]
fn declaration_surface_multilines_payload_union_with_named_fields() {
    let declare = Declare::new(
        intern("Type"),
        SpanId::INVALID,
        DeclareValue::Union {
            variants: vec![
                variant(generic(
                    "Primitive",
                    vec![("width", nominal("BigInt")), ("signed", nominal("Bool"))],
                )),
                variant(generic(
                    "Record",
                    vec![
                        ("name", nominal("String")),
                        (
                            "fields",
                            generic("List", vec![("NamedTy", nominal("NamedTy"))]),
                        ),
                    ],
                )),
                variant(generic("Ptr", vec![("inner", nominal("Type"))])),
            ],
        },
    );

    assert_eq!(
        declare.surface_text(),
        "Type is Primitive(width BigInt, signed Bool)\n     or Record(name String, fields List(NamedTy))\n     or Ptr(inner Type)"
    );
}
