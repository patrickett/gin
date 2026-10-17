use ast::{BindValue, Expr};
use parser::parse_from_str;

#[test]
fn emit_preserves_code_and_runtime_argument_expressions() {
    let parsed =
        parse_from_str("run(value Word) Word: return #emit(build(), Inputs(value: value))\n");
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let BindValue::Body { ret, .. } = &parsed.defs.values().next().unwrap().value else {
        panic!("expression body")
    };
    let body = &ret.values[0];
    assert!(
        matches!(&body.value, Expr::Emit { code, arguments } if matches!(code.value, Expr::FnCall(_)) && matches!(arguments.value, Expr::TagCall(_)))
    );
}

#[test]
fn emit_traversal_visits_both_stages() {
    let parsed = parse_from_str("run(value Word) Word: return #emit(code, [value: value])\n");
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let BindValue::Body { ret, .. } = &parsed.defs.values().next().unwrap().value else {
        panic!("expression body")
    };
    let body = &ret.values[0];
    let mut children = Vec::new();
    let _ = ast::folder::walk_typed_expr_children(&body.value, &mut |child| {
        children.push(child);
        std::ops::ControlFlow::Continue(())
    });
    assert_eq!(children.len(), 2);
}
