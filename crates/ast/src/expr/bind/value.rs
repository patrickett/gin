use crate::expr::Typed;
use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindValue {
    Expr(Box<Typed<Expr>>),
    Body {
        exprs: Vec<Typed<Expr>>,
        ret: Return,
    },
    /// External function declaration — no body, provided by the C runtime or linker.
    Extern,
}
