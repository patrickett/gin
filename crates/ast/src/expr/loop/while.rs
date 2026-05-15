use crate::span::{SpanId, Spanned};

use crate::expr::Expr;

/// While loop: loop while a condition holds.
///
/// ```gin
/// main:
///     while x < 10
///     loop
/// return
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhileLoop {
    pub cond: Box<Spanned<Expr>>,
    pub exprs: Vec<Spanned<Expr>>,
    pub span: SpanId,
}
