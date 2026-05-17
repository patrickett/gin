use crate::expr::Expr;
use crate::expr::Typed;
use crate::span::SpanId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FormatPart {
    Text(String),
    Expr(Box<Typed<Expr>>, SpanId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatString {
    pub parts: Vec<FormatPart>,
}
