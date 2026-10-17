use super::normal_expr;
use crate::FileId;
use crate::test_support::push_expr;
use crate::typed::{ExprId, TypedExprKind, TypedFileAst};
use ast::{ConstValue, Literal, NormalExpr, SpanTable};
use internment::Intern;

fn binding_value(unassigned: bool) -> Option<NormalExpr> {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let body = push_expr(&mut typed, TypedExprKind::Lit(Literal::Int(7.into())));
    assert_eq!(body, ExprId(0));
    let binding = push_expr(
        &mut typed,
        TypedExprKind::Bind {
            name: Intern::from_ref("value"),
            stmts: vec![],
            body,
            unassigned,
        },
    );
    normal_expr(&typed, None, &Default::default(), binding)
}

#[test]
fn unassigned_binding_does_not_read_placeholder_expression() {
    assert_eq!(binding_value(true), None);
}

#[test]
fn initialized_binding_preserves_initializer_at_zero() {
    assert_eq!(
        binding_value(false),
        Some(NormalExpr::Value(ConstValue::Int(7.into())))
    );
}
