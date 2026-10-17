use super::*;
use ast::{TyArg, ty::NamedTypeInstance};

fn container(arguments: Vec<(&str, TyArg)>) -> Ty {
    let name = Intern::from_ref("Container");
    Ty::Named {
        name,
        instance: NamedTypeInstance {
            declaration: ast::ty::TypeId { file: 0, name },
            arguments: arguments
                .into_iter()
                .map(|(name, arg)| (Intern::from_ref(name), arg))
                .collect(),
        },
    }
}

fn count(value: i128) -> TyArg {
    TyArg::Const(value.into())
}

#[test]
fn expected_return_rejects_conflicting_arguments() {
    let typed = TypedFileAst::new(crate::FileId(0), ast::SpanTable::new());
    for (returned, expected) in [
        (
            container(vec![("count", count(4))]),
            container(vec![("count", count(8))]),
        ),
        (
            container(vec![("element", TyArg::Type(Box::new(Ty::Unit)))]),
            container(vec![(
                "element",
                TyArg::Type(Box::new(Ty::Array {
                    elem: Box::new(Ty::Unit),
                    size: 4.into(),
                })),
            )]),
        ),
        (
            container(vec![("element", TyArg::Type(Box::new(Ty::Unit)))]),
            container(vec![("element", count(4))]),
        ),
        (
            container(vec![("count", count(4))]),
            container(vec![("other", count(4))]),
        ),
        (
            container(vec![
                ("left", TyArg::Const(NormalExpr::Var(Intern::from_ref("n")))),
                (
                    "right",
                    TyArg::Const(NormalExpr::Var(Intern::from_ref("n"))),
                ),
            ]),
            container(vec![("left", count(4)), ("right", count(8))]),
        ),
    ] {
        assert!(
            substitution_from_expected_return(&typed, &returned, Some(&expected)).is_none(),
            "{returned:?} -> {expected:?}"
        );
        assert!(
            infer_return_type_from_expected(&typed, &returned, Some(&expected)).is_none(),
            "{returned:?} -> {expected:?}"
        );
    }
}

#[test]
fn expected_return_infers_consistent_type_and_const_parameters() {
    let typed = TypedFileAst::new(crate::FileId(0), ast::SpanTable::new());
    let returned = container(vec![
        (
            "element",
            TyArg::Type(Box::new(Ty::Opaque(Intern::from_ref("t")))),
        ),
        ("left", TyArg::Const(NormalExpr::Var(Intern::from_ref("n")))),
        (
            "right",
            TyArg::Const(NormalExpr::Var(Intern::from_ref("n"))),
        ),
    ]);
    let expected = container(vec![
        ("element", TyArg::Type(Box::new(Ty::Unit))),
        ("left", count(4)),
        ("right", count(4)),
    ]);
    assert_eq!(
        infer_return_type_from_expected(&typed, &returned, Some(&expected)),
        Some(expected.clone())
    );
    let subst = substitution_from_expected_return(&typed, &returned, Some(&expected)).unwrap();
    assert_eq!(subst.apply_to_ty(&returned), expected);
}

#[test]
fn expected_return_requires_the_same_constructor_and_parameter_shape() {
    let typed = TypedFileAst::new(crate::FileId(0), ast::SpanTable::new());
    let returned = container(vec![("count", count(4))]);
    let mut other_declaration = returned.clone();
    let Ty::Named { instance, .. } = &mut other_declaration else {
        unreachable!()
    };
    instance.declaration.file = 1;
    let wrong_arity = container(vec![("count", count(4)), ("extra", count(4))]);
    for expected in [
        None,
        Some(&Ty::Unit),
        Some(&other_declaration),
        Some(&wrong_arity),
    ] {
        assert!(substitution_from_expected_return(&typed, &returned, expected).is_none());
        assert!(infer_return_type_from_expected(&typed, &returned, expected).is_none());
    }
}

#[test]
fn allocation_length_fallback_preserves_qualified_symbols() {
    for name in ["count", "sizes.count", "Sizes.dimensions.count"] {
        let mut parts = name.split('.');
        let expression = Expr::FnCall(ast::FnCall {
            syntax: ast::CallSyntax::Path,
            definition: None,
            path: ast::Spanned::new(
                ast::ModPath::new(
                    Intern::from_ref(parts.next().unwrap()),
                    parts.map(Intern::from_ref).collect(),
                ),
                SpanId::INVALID,
            ),
            args: None,
        });
        let expected = NormalExpr::Var(Intern::from_ref(name));
        assert_eq!(
            size_expr_to_normal(&expression, SizeCallName::Mangled, false),
            Some(expected.clone())
        );
        let arithmetic = Expr::Binary(ast::Binary {
            op: ast::BinOp::Multiply,
            lhs: Box::new(ast::Typed::infer(expression, SpanId::INVALID)),
            rhs: Box::new(ast::Typed::infer(
                Expr::Lit(Literal::Int(2_i128.into())),
                SpanId::INVALID,
            )),
        });
        assert_eq!(
            size_expr_to_normal(&arithmetic, SizeCallName::Mangled, false),
            Some(NormalExpr::Mul(Box::new(expected), Box::new(2_i128.into())))
        );
    }
}

#[test]
fn allocation_length_fallback_accepts_only_supported_expression_forms() {
    assert_eq!(
        size_expr_to_normal(
            &Expr::Lit(Literal::Int(i256::I256::from(usize::MAX as u128))),
            SizeCallName::Mangled,
            false
        ),
        Some(NormalExpr::from(i256::I256::from(usize::MAX as u128)))
    );
    for expression in [
        Expr::Lit(Literal::Float(ast::HashFloat(1.5))),
        Expr::Lit(Literal::Float(ast::HashFloat(f64::NAN))),
        Expr::Lit(Literal::String("4".to_string())),
        Expr::FnCall(ast::FnCall {
            syntax: ast::CallSyntax::Path,
            definition: None,
            path: ast::Spanned::new(
                ast::ModPath::new(Intern::from_ref("count"), Vec::new()),
                SpanId::INVALID,
            ),
            args: Some(Vec::new()),
        }),
        Expr::Binary(ast::Binary {
            op: ast::BinOp::Divide,
            lhs: Box::new(ast::Typed::infer(
                Expr::Lit(Literal::Int(4_i128.into())),
                SpanId::INVALID,
            )),
            rhs: Box::new(ast::Typed::infer(
                Expr::Lit(Literal::Int(2_i128.into())),
                SpanId::INVALID,
            )),
        }),
    ] {
        assert_eq!(
            size_expr_to_normal(&expression, SizeCallName::Mangled, false),
            None
        );
    }
}
