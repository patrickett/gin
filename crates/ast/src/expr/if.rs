use crate::expr::r#return::Return;
use crate::expr::{Condition, Expr, Typed};
use crate::span::SubSpan;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IfExpr {
    pub condition: Condition,
    pub body: Vec<Typed<Expr>>,
    pub ret: Return,
    pub body_span: SubSpan,
}
