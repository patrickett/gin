use ast::folder::{Visitor, walk_expr};
use ast::{Expr, Literal, SpanId, Typed};
use std::ops::ControlFlow;

#[test]
fn fixed_array_allocation_visits_initializer_and_size() {
    struct Literals(Vec<i256::I256>);

    impl Visitor for Literals {
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            if let Expr::Lit(Literal::Int(value)) = expr {
                self.0.push(*value);
            }
            walk_expr(self, expr)
        }
    }

    let expr = Expr::FixedArrayAlloc {
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

#[test]
fn emission_visitor_traverses_code_and_arguments() {
    struct Nodes(usize);

    impl Visitor for Nodes {
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            self.0 += 1;
            walk_expr(self, expr)
        }
    }

    let expr = Expr::Emit {
        code: Box::new(Typed::infer(
            Expr::Lit(Literal::String("spec".into())),
            SpanId::INVALID,
        )),
        arguments: Box::new(Typed::infer(
            Expr::Lit(Literal::Int(1.into())),
            SpanId::INVALID,
        )),
    };
    let mut nodes = Nodes(0);
    let _ = nodes.visit_expr(&expr);
    assert_eq!(nodes.0, 3);
}
