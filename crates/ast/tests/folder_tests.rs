use ast::folder::{Visitor, walk_expr};
use ast::{Expr, Literal, SpanId, Typed};
use std::ops::ControlFlow;

#[test]
fn tuple_allocation_visits_initializer_and_size() {
    struct Literals(Vec<i256::I256>);

    impl Visitor for Literals {
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            if let Expr::Lit(Literal::Int(value)) = expr {
                self.0.push(*value);
            }
            walk_expr(self, expr)
        }
    }

    let expr = Expr::TupleAlloc {
        init: Box::new(Typed::infer(
            Expr::Lit(Literal::Int(1.into())),
            SpanId::INVALID,
        )),
        size: Box::new(Typed::infer(
            Expr::Lit(Literal::Int(2.into())),
            SpanId::INVALID,
        )),
    };
    let mut literals = Literals(Vec::new());

    let _ = literals.visit_expr(&expr);

    assert_eq!(literals.0, vec![1.into(), 2.into()]);
}
