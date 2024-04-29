use super::parser::lexer::{token::Token, Location};
use core::fmt;

#[derive(Debug)]
pub enum CompilerError {
    /// Should only happen within the lexer/parser
    UnknownToken(Location, Token),

    InvalidRange(Location),
    CannotCallNonExpr(Location),
}

// Implement the Display trait for your custom error enum
impl fmt::Display for CompilerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompilerError::UnknownToken(loc, token) => {
                write!(
                    f,
                    "Unknown token {:#?} at location {}",
                    token,
                    loc.to_string()
                )
            }
            CompilerError::CannotCallNonExpr(loc) => {
                write!(
                    f,
                    "Can only call expressions. Issue at location {}",
                    loc.to_string()
                )
            }
            CompilerError::InvalidRange(loc) => {
                write!(f, "Invalid range at location {}", loc.to_string())
            }
        }
    }
}
