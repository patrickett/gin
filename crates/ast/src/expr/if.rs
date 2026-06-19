use crate::TypeExpr;
use crate::expr::Expr;
use crate::expr::Typed;
use crate::expr::r#return::Return;
use crate::span::Spanned;
use crate::span::SubSpan;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IfExpr {
    pub subject: Box<Typed<Expr>>,
    /// Parsed `is …` pattern, or `None` for bare `if expr` resolved through `Happy.value`.
    pub pattern: Option<Box<Spanned<TypeExpr>>>,
    pub body: Vec<Typed<Expr>>,
    pub ret: Return,
    pub body_span: SubSpan,
}
