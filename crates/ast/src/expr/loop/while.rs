use crate::expr::Expr;
use crate::expr::Typed;
use crate::span::SpanId;

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
    pub cond: Box<Typed<Expr>>,
    pub exprs: Vec<Typed<Expr>>,
    pub span: SpanId,
}
