use crate::TypeExpr;
use crate::expr::Expr;
use crate::expr::Typed;
use crate::span::{SpanId, Spanned};

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
    pub span: SpanId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhenArm {
    /// Boolean condition: `<condition> then <body>`
    Cond {
        condition: Box<Typed<Expr>>,
        body: Box<Typed<Expr>>,
        span: SpanId,
    },
    /// Pattern match: `is <type> then <body>` — structural type [`TypeExpr`] on the pattern field.
    Is {
        pattern: Box<Spanned<TypeExpr>>,
        body: Box<Typed<Expr>>,
        span: SpanId,
    },
    /// Fallthrough: `else <body>`
    Else(Box<Typed<Expr>>, SpanId),
}

impl std::hash::Hash for WhenArm {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Cond {
                condition,
                body,
                span,
            } => {
                condition.hash(state);
                body.hash(state);
                span.hash(state);
            }
            Self::Is {
                pattern,
                body,
                span,
            } => {
                pattern.hash(state);
                body.hash(state);
                span.hash(state);
            }
            Self::Else(body, span) => {
                body.hash(state);
                span.hash(state);
            }
        }
    }
}
