use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CondBranchEnding {
    Return(Expr),
    Yield(Expr),
    NoEnding,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IfExpr {
    // TODO: Implement if expression fields
    pub conds: Vec<Expr>,
    pub block: Vec<Expr>,
    pub ending: Box<CondBranchEnding>,
}

// If in gin is a non-exhaustive check with binding.
// Mainly used like so:
// ```gin
// main:
//     if x
//     is String
//         x -- is a String here
//     is UserIdentifier
//         x -- is a UserIdentifier here
//
// return
// ```
// or single line
// // ```gin
// main:
//     if x is String
//         x -- is a String here
// return
// ```
//
// Conditional branches like `if` can optionally end with a `return` or `yield`
//
// Note: there are no else branches, for exhaustive checks use `when`
pub fn if_expr<'t, I>() -> impl Parser<'t, I, IfExpr, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    // use Token::*;

    todo()
}
