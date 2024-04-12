// use std::collections::HashMap;

use crate::gin_type::GinType;

use super::Expr;

#[derive(Debug, Clone, PartialEq)]
pub enum ControlFlow {
    // boolean condition, body of condition, returntype
    WhenConditional(Expr, Vec<Expr>, GinType),
    // variable name
    WhenTypeIs(String),
}
