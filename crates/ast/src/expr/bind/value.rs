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
    /// A variable declared with a type but no assigned value yet.
    /// e.g. `value Int` (on its own line) declares `value` of type `Int` without assignment.
    /// Assignment happens later via `name: value` which becomes `TypedExprKind::Reassign`.
    Unassigned,
}
