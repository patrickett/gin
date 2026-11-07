use std::fmt;

use diagnostic::Diagnostic;
use diagnostic::code::SyntaxCode;
use ast::Span;

/// Configurable work budgets, not Gin language limits.
///
/// Defaults are provisional headroom for handwritten and generated files.
/// Recursive ownership of this prototype's boxed AST still requires depth caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLimits {
    pub source_bytes_max: u32,
    pub tokens_max: u32,
    pub parentheses_max: u16,
    pub expression_depth_max: u16,
    pub body_depth_max: u16,
    pub diagnostics_max: u32,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            source_bytes_max: 16 * 1024 * 1024,
            tokens_max: 1_000_000,
            parentheses_max: 128,
            expression_depth_max: 256,
            body_depth_max: 64,
            diagnostics_max: 100,
        }
    }
}

impl ParseLimits {
    pub(crate) fn validate(self) -> Result<(), ParseFailure> {
        if self.diagnostics_max == 0 {
            return Err(ParseFailure::InvalidLimits(
                "diagnostics_max must be positive",
            ));
        }
        if self.expression_depth_max == 0 {
            return Err(ParseFailure::InvalidLimits(
                "expression_depth_max must be positive",
            ));
        }
        // Each nested body adds binding/statement/block ownership around an expression.
        // Do not permit options to remove the boxed-tree ownership safety guard.
        let ownership_depth =
            u32::from(self.expression_depth_max) + 4 * u32::from(self.body_depth_max);
        if ownership_depth > 512 {
            return Err(ParseFailure::InvalidLimits(
                "expression_depth_max + 4 * body_depth_max must not exceed 512",
            ));
        }
        Ok(())
    }
}

/// Cancellation is checked between lexer tokens and parser/recovery steps.
///
/// This is cooperative cancellation, not preemption within a single lexer token.
#[derive(Clone, Copy, Default)]
pub struct ParseOptions<'cancel> {
    pub limits: ParseLimits,
    pub is_cancelled: Option<&'cancel dyn Fn() -> bool>,
}

impl ParseOptions<'_> {
    pub(crate) fn checkpoint(self, span: Span) -> Result<(), ParseFailure> {
        if self.is_cancelled.is_some_and(|check| check()) {
            return Err(ParseFailure::Cancelled { span });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseLimit {
    SourceBytes,
    Tokens,
    Parentheses,
    ExpressionDepth,
    BodyDepth,
    Diagnostics,
    /// Spans issued for one file; bounded by the file-indexed id range.
    Spans,
}

/// Refusal to finish a parse. Syntax and lexical errors are *not* failures here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseFailure {
    Cancelled {
        span: Span,
    },
    LimitExceeded {
        limit: ParseLimit,
        maximum: u32,
        span: Span,
    },
    InvalidLimits(&'static str),
}

impl ParseFailure {
    pub(crate) fn diagnostic(self) -> Diagnostic {
        let span = match &self {
            Self::Cancelled { span } | Self::LimitExceeded { span, .. } => *span,
            Self::InvalidLimits(_) => Span::new(0, 0),
        };
        Diagnostic::new(SyntaxCode::UnexpectedToken, self.to_string()).at_span(span)
    }
}

impl fmt::Display for ParseFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled { .. } => formatter.write_str("source parsing cancelled"),
            Self::LimitExceeded { limit, maximum, .. } => {
                write!(
                    formatter,
                    "source parser {limit:?} limit exceeded ({maximum})"
                )
            }
            Self::InvalidLimits(message) => write!(formatter, "invalid parser limits: {message}"),
        }
    }
}

impl std::error::Error for ParseFailure {}
