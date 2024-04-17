// use std::collections::HashMap;

use crate::ngin::{
    gin_type::GinType,
    parser::module::{expression::Expr, Node},
};

#[derive(Debug, Clone, PartialEq)]
pub enum ControlFlow {
    // condition, if true body, else body
    If(Expr, Vec<Node>, Option<Vec<Node>>),

    // boolean condition, body of condition, returntype
    WhenConditional(Expr, Vec<Expr>, GinType),
    // variable name
    WhenTypeIs(String),

    Return(Expr),
}
