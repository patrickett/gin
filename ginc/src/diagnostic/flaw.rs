use std::{fs, ops::Range, path::PathBuf};

use crate::{diagnostic::Printable, frontend::Token};

#[derive(Debug)]
pub enum GincFlaw<'a> {
    ParseError(Vec<(Vec<Rich<'a, Token<'a>>>, PathBuf)>),
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
impl Printable for GincFlaw<'_> {
    fn print(&self) {
        match self {
            GincFlaw::ParseError(errors) => {
                for (errors, path) in errors {
                    let filename = path.to_str().expect("msg").to_string();
                    let src_txt = fs::read_to_string(path).expect("msg");

                    let mut cache = ariadne::sources([(filename.clone(), src_txt)]);

                    for err in errors.iter() {
                        let span = err.span();
                        let start = span.start();
                        let end = span.end();

                        let ariadne_span = (filename.clone(), Range { start, end });
                        let msg = format!("{:?}", err);

                        let report = Report::build(
                            ReportKind::Custom("flaw", ariadne::Color::Red),
                            ariadne_span.clone(),
                        )
                        .with_message(msg)
                        .with_label(Label::new(ariadne_span).with_message("here"))
                        .finish();

                        report.eprint(&mut cache).unwrap();
                    }
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

use ariadne::{Label, Report, ReportKind};
use chumsky::{error::Rich, span::Span};

/// Print a list of `chumsky` parse errors using ariadne for coloured output.
///
/// This helper is meant to be called from the parser when it encounters
/// errors.  It takes ownership of the error vector and prints each one to
/// stderr.
pub fn print_parse_errors(
    errors: &Vec<chumsky::prelude::Rich<'static, Token<'static>>>,
    src_txt: &str,
    filename: String,
) {
    let mut cache = ariadne::sources([(filename.clone(), src_txt)]);

    for err in errors.iter() {
        let span = err.span();
        let start = span.start();
        let end = span.end();

        let ariadne_span = (filename.clone(), Range { start, end });
        let msg = format!("{:?}", err);

        let report = Report::build(
            ReportKind::Custom("flaw", ariadne::Color::Red),
            ariadne_span.clone(),
        )
        .with_message(msg)
        .with_label(Label::new(ariadne_span).with_message("here"))
        .finish();

        report.eprint(&mut cache).unwrap();
    }
}
