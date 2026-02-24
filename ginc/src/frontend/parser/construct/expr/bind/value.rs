use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindValue {
    Expr(Box<Expr>),
    Body { exprs: Vec<Expr>, ret: Return },
}
