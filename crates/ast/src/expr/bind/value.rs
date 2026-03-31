use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindValue {
    Expr(Box<Spanned<Expr>>),
    Body {
        exprs: Vec<Spanned<Expr>>,
        ret: Return,
    },
    /// External function declaration — no body, provided by the C runtime or linker.
    Extern,
}
