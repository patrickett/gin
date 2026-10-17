use crate::Pattern;
use crate::expr::{Condition, Expr, Typed};
use crate::span::{Spanned, SubSpan};

/// Ordered conditional cases with an optional shared subject.
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
        condition: Condition,
        body: Box<Typed<Expr>>,
        arm_span: SubSpan,
    },
    /// Shared-subject pattern: `<pattern> then <body>`.
    Is {
        pattern: Box<Spanned<Pattern>>,
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
