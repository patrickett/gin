//! Formats Gin source and verifies syntax, spelling, comments and idempotence.
//!
//! Only `lexer`, `parser` and `ast` are used. A file is formatted only
//! when it parses without flaws, and the result is checked before it is returned: it must
//! parse, to the same tree and the same spelling, keep every comment where it was among the
//! code, and format to itself.

pub(crate) mod print;
mod scan;
mod wrap;
pub use wrap::{Cell, breakable_lines};
pub mod cli;
pub mod verify;

use diagnostic::Diagnostic;

pub use print::INDENT;

/// How the types of fields in a `has` body are lined up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// One space after the name.
    Off,
    /// Pad the types of single-line fields on adjacent lines to one column.
    Adjacent,
    /// As `Adjacent`, but a line of attributes (`#private`) between two fields does not end the run.
    ThroughAttributes,
}

/// Style choices the library exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub align: Align,
    /// Columns of code after which lines are broken where the grammar allows; `None` never breaks.
    pub width: Option<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Self { align: Align::Adjacent, width: Some(100) }
    }
}

#[derive(Debug)]
pub enum FormatError {
    /// The parser reported flaws, or could not finish; the source is not formatted.
    Flaws(Vec<Diagnostic>),
    /// The printer produced something that breaks a guarantee. A bug in this tool; the source
    /// is left as it was.
    Unverified(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Flaws(diagnostics) => write!(f, "{} syntax flaw(s)", diagnostics.len()),
            Self::Unverified(why) => write!(f, "formatter bug, file left untouched: {why}"),
        }
    }
}

fn parse(source: &str) -> Result<ast::SourceFile, FormatError> {
    parser::parse_strict(source).map_err(FormatError::Flaws)
}

/// Print without verifying. Exposed for tests of the guarantees.
pub fn print_unverified(source: &str, options: Options) -> Result<String, FormatError> {
    let file = parse(source)?;
    Ok(print::print(source, &file, options))
}

pub fn format(source: &str) -> Result<String, FormatError> {
    format_with(source, Options::default())
}

pub fn format_with(source: &str, options: Options) -> Result<String, FormatError> {
    format_detailed(source, options).map(|(text, _)| text)
}

/// The formatted text, and whether wrapping had to be given up for this file because the wrapped
/// text broke a guarantee (the unwrapped text is returned then). The tests require `false`.
pub fn format_detailed(source: &str, options: Options) -> Result<(String, bool), FormatError> {
    match verified(source, options) {
        Ok(text) => Ok((text, false)),
        Err(FormatError::Unverified(_)) if options.width.is_some() => {
            verified(source, Options { width: None, ..options }).map(|text| (text, true))
        }
        Err(error) => Err(error),
    }
}

fn verified(source: &str, options: Options) -> Result<String, FormatError> {
    let output = print_unverified(source, options)?;
    let unverified = |why: String| FormatError::Unverified(why);
    verify::check_semantics(source, &output).map_err(unverified)?;
    verify::check_comments(source, &output).map_err(unverified)?;
    let again = print_unverified(&output, options)
        .map_err(|_| FormatError::Unverified("output does not parse".to_owned()))?;
    if again != output {
        return Err(FormatError::Unverified("output is not stable under a second pass".to_owned()));
    }
    Ok(output)
}
