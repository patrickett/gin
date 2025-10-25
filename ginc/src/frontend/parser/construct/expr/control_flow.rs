// for block loop
// if block return yield

use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum CondBranchEnding<'src> {
    Return(Expr<'src>),
    Yield(Expr<'src>),
    NoEnding,
}

#[derive(Debug, Clone)]
pub enum ControlFlow<'src> {
    /// An if expression allows you to branch your code depending on conditions.
    /// You provide a condition and then state, “If this condition is met, run this block of code.
    /// If the condition is not met, do not run this block of code.”
    ///
    /// ex.
    ///
    /// ```gin
    /// number: 3
    ///
    /// if number < 5 then
    ///     print('condition was true')
    /// ```
    If {
        // if (i = 5, i < 3) then
        //   print('hi')
        conds: Vec<Expr<'src>>,
        block: Vec<Expr<'src>>,
        ending: Box<CondBranchEnding<'src>>, // NOTE: I kinda don't want to support else branches
                                             // else_branch: Box<Expr<'src>>,
    },
    ///
    /// Example:
    /// ```gin
    /// main:
    ///     for item in items
    ///
    ///     loop
    /// return
    /// ```
    /// OR like a range
    /// ```gin
    /// main:
    ///     for i in 1..50
    ///     loop
    /// return
    /// ```
    ForIn {},
    While {},
}
