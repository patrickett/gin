use crate::expr::Expr;
use crate::span::{SpanId, Spanned};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FormatPart {
    Text(String),
    Expr(Box<Spanned<Expr>>, SpanId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatString {
    pub parts: Vec<FormatPart>,
    pub span: SpanId,
}
