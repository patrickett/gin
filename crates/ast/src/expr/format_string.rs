use crate::expr::Expr;
use crate::span::Spanned;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FormatPart {
    Text(String),
    Expr(Box<Spanned<Expr>>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatString {
    pub parts: Vec<FormatPart>,
}
