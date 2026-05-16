use crate::expr::Expr;
use crate::expr::Typed;
use crate::path::ModPath;
use crate::span::Spanned;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnCall {
    pub path: Spanned<ModPath>,
    pub args: Option<Vec<Typed<Expr>>>,
}
