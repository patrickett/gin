use crate::expr::Expr;
use crate::span::Spanned;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Return(pub Option<Box<Spanned<Expr>>>);
