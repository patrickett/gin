use crate::TypeExpr;
use crate::expr::Typed;
use crate::expr::r#return::Return;
use crate::span::Spanned;
use crate::span::SubSpan;

use crate::expr::Expr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IfCondition {
    Bool(Box<Typed<Expr>>),
    Pattern {
        subject: Box<Typed<Expr>>,
        /// Parsed `is …` pattern — structural [`TypeExpr`] (`Nominal` / `Qualified` / `Generic`).
        pattern: Box<Spanned<TypeExpr>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IfExpr {
    pub condition: IfCondition,
    pub body: Vec<Typed<Expr>>,
    pub ret: Return,
    pub body_span: SubSpan,
}
