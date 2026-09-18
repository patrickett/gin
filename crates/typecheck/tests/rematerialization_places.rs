use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

fn assert_runtime_return(source: &str, name: &str) {
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let returned = match typed.defs[&DefId(Intern::from_ref(name))].body {
        BindBody::Expr(value) | BindBody::Body { ret: Some(value), .. } => value,
        ref body => panic!("missing return: {body:?}"),
    };
    let TypedExprKind::Cast { expr, .. } = typed.exprs.kind[returned.as_usize()] else {
        panic!("expected cast return");
    };
    assert!(
        !matches!(typed.exprs.kind[expr.as_usize()], TypedExprKind::Rematerialize { .. }),
        "runtime value was replaced with {:?}", typed.exprs.kind[expr.as_usize()]
    );
}

#[test]
fn a_loop_rebind_is_not_a_constant_reaching_the_loop_exit() {
    assert_runtime_return(
        "Signed is in -9223372036854775808 to 9223372036854775807\nmain Signed:\n    value is in -9223372036854775808 to 9223372036854775807: 0\n    for index in 0 to 0\n        next is in -9223372036854775808 to 9223372036854775807: -1\n        value :: next\n    loop\nreturn value as Signed\n",
        "main",
    );
}

#[test]
fn a_parameter_does_not_inherit_another_callables_same_named_constant() {
    assert_runtime_return(
        "Word is in 0 to 18446744073709551615\nfirst Word:\n    value: 7\nreturn value as Word\nsecond(value is in 0 to 18446744073709551615) Word: value as Word\n",
        "second",
    );
}

#[test]
fn a_conditional_rebind_does_not_reuse_the_initializer_constant() {
    assert_runtime_return(
        "Word is in 0 to 18446744073709551615\nBool is True or False\nchoose(flag Bool) Word:\n    value is in 0 to 18446744073709551615: 0\n    next is in 0 to 18446744073709551615: when flag is True then 1 else value\n    value :: next\nreturn value as Word\n",
        "choose",
    );
}
