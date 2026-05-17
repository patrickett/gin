use crate::TypeExpr;
use crate::expr::Expr;
use crate::expr::Typed;
use crate::span::{Spanned, SubSpan};

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
    pub subject: Option<Box<Typed<Expr>>>,
    pub arms: Vec<WhenArm>,
    pub body_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhenArm {
    /// Boolean condition: `<condition> then <body>`
    Cond {
        condition: Box<Typed<Expr>>,
        body: Box<Typed<Expr>>,
        arm_span: SubSpan,
    },
    /// Pattern match: `is <type> then <body>` — structural type [`TypeExpr`] on the pattern field.
    Is {
        pattern: Box<Spanned<TypeExpr>>,
        body: Box<Typed<Expr>>,
        arm_span: SubSpan,
    },
    /// Fallthrough: `else <body>`
    Else(Box<Typed<Expr>>, SubSpan),
}

impl std::hash::Hash for WhenArm {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Cond {
                condition,
                body,
                arm_span,
            } => {
                condition.hash(state);
                body.hash(state);
                arm_span.hash(state);
            }
            Self::Is {
                pattern,
                body,
                arm_span,
            } => {
                pattern.hash(state);
                body.hash(state);
                arm_span.hash(state);
            }
            Self::Else(body, arm_span) => {
                body.hash(state);
                arm_span.hash(state);
            }
        }
    }
}
