use crate::parser::{Expr, Literal, Op};

pub fn evaluate_exprs(exprs: Vec<Expr>) -> Option<Literal> {
    println!("{:#?}", &exprs);
    let mut result: Option<Literal> = None;
    for expr in exprs {
        result = evaluate_expr(&expr);
    }
    result
}

fn evaluate_expr(expr: &Expr) -> Option<Literal> {
    match expr {
        Expr::Assignment { lhs, rhs } => {
            let rhs_value = evaluate_expr(rhs)?;
            // Perform the assignment operation here
            println!("Assigning value {:?} to variable {}", rhs_value, lhs);
            None
        }
        Expr::Literal(literal) => Some(literal.clone()),
        Expr::BinExpr { lhs, op, rhs } => {
            let lhs_value = evaluate_expr(lhs)?;
            let rhs_value = evaluate_expr(rhs)?;
            match op {
                Op::Add => perform_addition(&lhs_value, &rhs_value),
                Op::Sub => perform_subtraction(&lhs_value, &rhs_value),
                Op::Mul => perform_multiplication(&lhs_value, &rhs_value),
                // Implement the other operators as needed
                _ => None,
            }
        }
    }
}

fn perform_addition(lhs: &Literal, rhs: &Literal) -> Option<Literal> {
    match (lhs, rhs) {
        (Literal::Number(a), Literal::Number(b)) => Some(Literal::Number(a + b)),
        (Literal::String(a), Literal::String(b)) => Some(Literal::String(format!("{}{}", a, b))),
        _ => None,
    }
}

fn perform_subtraction(lhs: &Literal, rhs: &Literal) -> Option<Literal> {
    match (lhs, rhs) {
        (Literal::Number(a), Literal::Number(b)) => Some(Literal::Number(a - b)),
        _ => None,
    }
}

fn perform_multiplication(lhs: &Literal, rhs: &Literal) -> Option<Literal> {
    match (lhs, rhs) {
        (Literal::Number(a), Literal::Number(b)) => Some(Literal::Number(a * b)),
        _ => None,
    }
}
