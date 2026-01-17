use crate::{diagnostic::Printable, frontend::Token};

#[derive(Debug)]
pub enum GincFlaw {
    ParseError(Vec<chumsky::prelude::Rich<'static, Token<'static>>>),
    TypeError(String),
    BorrowError(String),
    IoError(String),
}

/// Custom flawing macro that formats messages consistently.
///
/// # Usage
///
/// The `flaw!` macro can be used throughout the codebase to emit formatted
/// flawing messages. It works similarly to Rust's standard macros like `println!`
/// but automatically prefixes all output with "flaw: " for consistency.
#[macro_export]
macro_rules! flaw {
    ($fmt:literal $(, $arg:expr)* $(,)?) => {
        eprintln!(concat!("flaw: ", $fmt), $($arg),*)
    };
}

// Implement Printable for GincFlaw
impl Printable for GincFlaw {
    fn print(&self) {
        match self {
            GincFlaw::ParseError(errors) => {
                eprintln!("Parse Error:");
                for error in errors {
                    flaw!("- {:#?}", error);
                }
            }
            GincFlaw::TypeError(msg) => {
                flaw!("Type Error: {}", msg);
            }
            GincFlaw::BorrowError(msg) => {
                flaw!("Borrow Error: {}", msg);
            }
            GincFlaw::IoError(msg) => {
                flaw!("IO Error: {}", msg);
            }
        }
    }
}
