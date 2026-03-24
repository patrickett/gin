use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindValue {
    Expr(Box<Expr>),
    Body { exprs: Vec<Expr>, ret: Return },
    /// External function declaration — no body, provided by the C runtime or linker.
    Extern,
}
