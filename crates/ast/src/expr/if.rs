use crate::expr::r#return::Return;
use crate::span::Spanned;
use crate::tag::Tag;

use crate::expr::Expr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IfCondition {
    Bool(Box<Spanned<Expr>>),
    Pattern {
        subject: Box<Spanned<Expr>>,
        tag: Tag,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IfExpr {
    pub condition: IfCondition,
    pub body: Vec<Spanned<Expr>>,
    pub ret: Return,
}
