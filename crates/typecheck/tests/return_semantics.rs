use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::ty::Ty;
use typecheck::{BindBody, DefId, FileId, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
}

fn def_body<'a>(typed: &'a TypedFileAst, name: &str) -> &'a BindBody {
    &typed.defs[&DefId(Intern::new(name.to_string()))].body
}

#[test]
fn block_with_bare_return_has_unit_return_and_empty_ret_slot() {
    let typed =
        check("Int is in 0...255\nwrite(value Int) Int: 0\nmain:\n    write(1)\n    return\n");

    let bind = def_body(&typed, "main");
    let (exprs, ret) = match bind {
        BindBody::Body { exprs, ret } => (exprs.as_slice(), ret),
        _ => panic!("expected block body for main"),
    };

    assert!(!exprs.is_empty());
    assert!(ret.is_none());
    assert!(matches!(
        typed.exprs.kind[exprs[0].as_usize()],
        typecheck::TypedExprKind::FnCall { .. }
    ));
    assert!(matches!(
        typed.defs[&DefId(Intern::from_ref("main"))].return_type,
        Ty::Unit
    ));
}

#[test]
fn block_with_explicit_return_keeps_return_expression() {
    let typed = check("main Int:\n    1\n    return 3\n");

    let bind = def_body(&typed, "main");
    let (exprs, ret) = match bind {
        BindBody::Body { exprs, ret } => (exprs.as_slice(), ret),
        _ => panic!("expected block body"),
    };

    assert_eq!(exprs.len(), 1);
    let return_expr = ret.expect("expected explicit return value");
    assert!(matches!(
        typed.exprs.kind[return_expr.as_usize()],
        typecheck::TypedExprKind::Lit(_)
    ));
    assert_eq!(
        typed.defs[&DefId(Intern::from_ref("main"))].return_type,
        Ty::Opaque(Intern::from_ref("Int"))
    );
}

#[test]
fn expression_body_infers_expression_type() {
    let typed = check("Int is in 0...255\nmain Int: 3\n");

    let bind = &typed.defs[&DefId(Intern::from_ref("main"))];
    let expr = match &bind.body {
        BindBody::Expr(expr) => *expr,
        _ => panic!("expected expression-bodied function"),
    };

    let expression_type = &typed.exprs.ty[expr.as_usize()];
    let return_type = &typed.defs[&DefId(Intern::from_ref("main"))].return_type;
    assert!(
        typed
            .type_registry
            .integer_validity_for_type(expression_type)
            .is_some()
    );
    assert!(
        typed
            .type_registry
            .integer_validity_for_type(return_type)
            .is_some()
    );
}
