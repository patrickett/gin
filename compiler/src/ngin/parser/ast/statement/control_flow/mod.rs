// use std::collections::HashMap;

use crate::ngin::{
    gin_type::GinType,
    parser::ast::{expression::Expr, Node},
};

#[derive(Debug, Clone, PartialEq)]
pub enum ControlFlow {
    // boolean condition, body of condition, returntype
    WhenConditional(Expr, Vec<Expr>, GinType),
    // variable name
    WhenTypeIs(String),

    Return(Expr),
}
