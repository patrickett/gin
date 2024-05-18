use super::parser::lexer::{token::Token, Location};

// #[derive(Debug)]
pub enum CompilerError {
    NoSuchFileOrDirectory,
    IO(std::io::Error),
    /// Token has position, String is path
    UnknownToken(Token, Option<String>),

    InvalidRange(Location),
    CannotCallNonExpr(Location),

    // TODO: add location
    UnexpectedEOF,
}

impl std::fmt::Debug for CompilerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompilerError::UnknownToken(token, path) => {
                let path_str = if let Some(path) = path {
                    format!("{}:", path)
                } else {
                    String::new()
                };

                write!(
                    f,
                    "unknown token `{}` at: {}:{}",
                    token.kind(),
                    path_str,
                    token.position().to_string()
                )
            }
            CompilerError::CannotCallNonExpr(loc) => {
                write!(
                    f,
                    "can only call expressions - issue at: {}",
                    loc.to_string()
                )
            }
            CompilerError::InvalidRange(loc) => {
                write!(f, "invalid range at: {}", loc.to_string())
            }
            CompilerError::IO(err) => write!(f, "io error: {}", err),
            CompilerError::UnexpectedEOF => write!(f, "unexpected eof"),
            CompilerError::NoSuchFileOrDirectory => write!(f, "no such file or directory"),
            // CompilerError::UnexpectedEOF(loc) => {
            //     write!(f, "unexpected end of file at: {}", loc.to_string())
            // }
        }
    }
}
