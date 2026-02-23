use crate::frontend::prelude::*;

/// While loop: loop while a condition is true
///
/// Example:
/// ```gin
/// main:
///     while x < 10
///     loop
/// return
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhileLoop {
    pub cond: Box<Expr>,
    pub exprs: Vec<Expr>,
}

// TODO: Implement while loop parser
// The `While` token needs to be added to the lexer first
