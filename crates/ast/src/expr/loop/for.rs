use crate::expr::Expr;
use crate::span::{SpanId, Spanned};

/// For-in loop: iterate over a range or collection
///
/// Example:
/// ```gin
/// main:
///     for item in items
///     loop
/// return
/// ```
/// OR like a range
/// ```gin
/// main:
///     for i in 1...50
///     loop
/// return
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForInLoop {
    /// Loop binder: a `for`-pattern-shaped [`Expr`] (see [`crate::pattern`]).
    /// Boxed to avoid a recursive `Expr` → `Loop` → `ForInLoop` → `Expr` layout.
    pub pat: Box<Spanned<Expr>>,
    // TODO: check and make sure it accepts expression that can be iterated
    pub iter: Box<Spanned<Expr>>,
    pub exprs: Vec<Spanned<Expr>>,
    pub span: SpanId,
}
