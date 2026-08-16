use ast::parameter::ParameterKind;
use ast::span::{SpanId, Spanned};
use ast::{Expr, Pattern, TypeExpr};
use ast_format::type_expr::PatternFormatExt;
use ast_format::type_expr::TypeExprFormatExt;
use internment::Intern;

fn pattern_name(name: &str) -> Spanned<Pattern> {
    Spanned {
        value: Pattern::Nominal(Intern::new(name.to_string()), SpanId::INVALID),
        span_id: SpanId::INVALID,
    }
}

fn list_cons(head: Spanned<Pattern>, tail: Spanned<Pattern>) -> Pattern {
    Pattern::ListCons {
        head: Box::new(head),
        tail: Box::new(tail),
    }
}

fn generic(name: &str, params: Vec<(&str, TypeExpr)>) -> TypeExpr {
    TypeExpr::Generic {
        name: Intern::new(name.to_string()),
        params: params
            .into_iter()
            .map(|(name, ty)| {
                (
                    Intern::new(name.to_string()),
                    ParameterKind::Tagged(Box::new(Spanned {
                        value: match ty {
                            TypeExpr::Nominal(name, _) => Expr::AnonymousTag(name),
                            TypeExpr::Literal(lit, _) => Expr::Lit(lit),
                            _ => Expr::Lit(ast::Literal::Number(0)),
                        },
                        span_id: SpanId::INVALID,
                    })),
                )
            })
            .collect(),
        param_spans: Vec::new(),
        span: SpanId::INVALID,
    }
}

#[test]
fn list_cons_variant_shape_surface_uses_js_rest_syntax() {
    let expr = list_cons(pattern_name("head"), pattern_name("tail"));

    assert_eq!(expr.format_variant_shape(), "[head, ...tail]");
}

#[test]
fn variant_shape_surface_keeps_field_names() {
    let expr = generic(
        "Primitive",
        vec![
            (
                "width",
                TypeExpr::Nominal(Intern::new("BigInt".to_string()), SpanId::INVALID),
            ),
            (
                "signed",
                TypeExpr::Nominal(Intern::new("Bool".to_string()), SpanId::INVALID),
            ),
        ],
    );

    assert_eq!(
        expr.format_variant_shape(),
        "Primitive(width BigInt, signed Bool)"
    );
}
