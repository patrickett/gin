use crate::expr::Expr;
use crate::pattern::Pattern;
use crate::span::Spanned;

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
    pub pat: Pattern,
    // TODO: check and make sure it accepts expression that can be iterated
    pub iter: Box<Spanned<Expr>>,
    pub exprs: Vec<Spanned<Expr>>,
}
