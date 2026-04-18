use crate::expr::Expr;
use crate::span::Spanned;

/// Exhaustive conditional expression.
///
/// Boolean condition form:
/// ```gin
/// when n % 15 = 0 then print("FizzBuzz")
///      n % 05 = 0 then print("Fizz")
///      n % 03 = 0 then print("Buzz")
///      else print(n)
/// ```
///
/// Pattern matching form:
/// ```gin
/// when value
///     is Some(x) then x
///     is None    then 0
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhenExpr {
    /// Subject expression for pattern matching (e.g., `when self`)
    /// None for condition-based when
    pub subject: Option<Box<Spanned<Expr>>>,
    pub arms: Vec<WhenArm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WhenArm {
    /// Boolean condition: `<condition> then <body>`
    Cond {
        condition: Box<Spanned<Expr>>,
        body: Box<Spanned<Expr>>,
    },
    /// Pattern match: `is <tag> then <body>` — [`Expr::IsPattern`] on the pattern field.
    Is {
        pattern: Box<Spanned<Expr>>,
        body: Box<Spanned<Expr>>,
    },
    /// Fallthrough: `else <body>`
    Else(Box<Spanned<Expr>>),
}
