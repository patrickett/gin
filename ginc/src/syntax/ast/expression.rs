use super::Node;
use crate::syntax::token::Literal;

#[derive(Debug, Clone, PartialEq)]
/// Expressions are things that can be evaluated to a value.
pub enum Expr {
    Cond(If),
    Call(FunctionCall),
    Arithmetic(Box<ArithmeticExpr>),
    Relational(Box<RelationalExpr>),
    Literal(Literal),
}

#[derive(Debug, Clone, PartialEq)]
pub struct If {
    cond: Box<RelationalExpr>,
    true_body: Vec<Node>,
    false_body: Option<Vec<Node>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionCall {
    name: String,
    arg: Option<Box<Expr>>,
}

impl FunctionCall {
    pub fn new(name: String, arg: Option<Box<Expr>>) -> Self {
        Self { name, arg }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RelationalExpr {
    LessThan { lhs: Expr, rhs: Expr },
    LessThanOrEqualTo { lhs: Expr, rhs: Expr },
    GreaterThan { lhs: Expr, rhs: Expr },
    GreaterThanOrEqualTo { lhs: Expr, rhs: Expr },
    Equals { lhs: Expr, rhs: Expr },
    NotEquals { lhs: Expr, rhs: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArithmeticExpr {
    Add { lhs: Expr, rhs: Expr },
    Sub { lhs: Expr, rhs: Expr },
    Div { lhs: Expr, rhs: Expr },
    Mul { lhs: Expr, rhs: Expr },
}
