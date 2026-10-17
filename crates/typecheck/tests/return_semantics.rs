mod support;

use internment::Intern;
use support::{check, flaw_codes};
use typecheck::ty::Ty;
use typecheck::{BindBody, DefId, TypedFileAst};

fn def_body<'a>(typed: &'a TypedFileAst, name: &str) -> &'a BindBody {
    &typed.defs[&DefId::named(Intern::new(name.to_string()))].body
}

#[test]
fn block_with_bare_return_has_unit_return_and_empty_ret_slot() {
    let typed = check(
        "Int is in 0 to 255\nwrite(value Int) Int: return 0\nmain:\n    write(1)\n    return\n",
    );

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
        typed.defs[&DefId::named(Intern::from_ref("main"))].return_type,
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
        typed.defs[&DefId::named(Intern::from_ref("main"))].return_type,
        Ty::Opaque(Intern::from_ref("Int"))
    );
}

#[test]
fn expression_body_infers_expression_type() {
    let typed = check("Int is in 0 to 255\nmain Int: 3\n");

    let bind = &typed.defs[&DefId::named(Intern::from_ref("main"))];
    let expr = match &bind.body {
        BindBody::Expr(expr) => *expr,
        _ => panic!("expected expression-bodied function"),
    };

    let expression_type = &typed.exprs.ty[expr.as_usize()];
    let return_type = &typed.defs[&DefId::named(Intern::from_ref("main"))].return_type;
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

#[test]
fn unit_block_functions_can_finish_without_return() {
    for body in [
        "write(value mut Byte):\n    value:: replace(deref value, 7)\n",
        "write(value mut Byte, flag Byte):\n    when flag = 1 then return\n    value:: replace(deref value, 7)\n",
        "write(value mut Byte, flag Byte):\n    when flag = 1 then\n        value:: replace(deref value, 7)\n",
        "write(value mut Byte):\n    local():\n        value:: replace(deref value, 7)\n    local()\n",
        "write(value mut Byte, flag Byte):\n    local(condition Byte):\n        when condition = 1 then return\n        value:: replace(deref value, 7)\n    local(flag)\n",
        "write(value mut Byte, flag Byte):\n    local(condition Byte):\n        when condition = 1 then\n            value:: replace(deref value, 7)\n    local(flag)\n",
    ] {
        let source = format!(
            "Byte is in 0 to 255\nByte has finish(self): return\nreplace(previous Byte, next Byte) Byte: return next\n{body}"
        );
        let typed = check(&source);
        assert!(
            typed.all_flaws().is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
        assert_eq!(
            typed.defs[&DefId::named(Intern::from_ref("write"))].return_type,
            Ty::Unit,
            "{source}"
        );
    }
}

#[test]
fn block_functions_with_values_still_require_return() {
    for body in [
        "read Byte:\n    7\n",
        "read() Byte:\n    7\n",
        "read(value Byte) Byte:\n    value\n",
        "read(value Byte) Byte:\n    local(input Byte) Byte:\n        input\n    return local(value)\n",
    ] {
        let source = format!("Byte is in 0 to 255\nByte has finish(self): return\n{body}");
        let typed = check(&source);
        assert!(
            flaw_codes(&typed)
                .iter()
                .any(|code| code == "type-missing-return"),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn inferred_value_returns_cannot_fall_through() {
    for body in [
        "read(flag Byte):\n    when flag = 1 then return 7\n    flag\n",
        "read(flag Byte):\n    local(condition Byte):\n        when condition = 1 then return 7\n        condition\n    return local(flag)\n",
    ] {
        let source = format!("Byte is in 0 to 255\nByte has finish(self): return\n{body}");
        let typed = check(&source);
        assert!(
            flaw_codes(&typed)
                .iter()
                .any(|code| code == "type-return-mismatch"),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}
