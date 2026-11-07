//! Recovering Gin parser shared by compiler and editor consumers.

mod annotation;
mod cursor;
mod declaration;
mod domain;
mod expr;
mod file;
mod group;
mod import;
mod layout;
mod operators;
mod options;
mod pattern;
mod types;
mod when;

use cursor::Cursor;
use diagnostic::{Category, Diagnostic};
use ast::{SourceFile, SpanTable};

pub use options::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};

/// A syntax snapshot, including recoverable errors, suitable for editor queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseOutput {
    pub file: SourceFile,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParseOutput {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.category == Category::Flaw)
    }

    /// Strict consumers must not compile or format recovered syntax as valid input.
    pub fn into_result(self) -> Result<SourceFile, Vec<Diagnostic>> {
        if self.has_errors() {
            Err(self.diagnostics)
        } else {
            Ok(self.file)
        }
    }
}

/// Parse source without resolution, staging, or desugaring.
///
/// Ordinary lexical/syntax errors return a recovered tree and diagnostics.
/// `Err` is reserved for cancellation, invalid budgets, or exceeded budgets.
///
/// ```
/// let output = parser::parse("answer := 42\n").expect("parse completed");
/// assert!(!output.has_errors());
/// assert_eq!(output.file.items.len(), 1);
/// assert_eq!(output.file.text.as_ref(), "answer := 42\n");
/// let hit = output.file.node_at_offset(2).expect("UTF-8 byte insertion point");
/// assert_eq!(hit.node.span.extract(&output.file.text), "answer");
/// assert_eq!(output.file.declaration_outline().next().unwrap().name, "answer");
/// ```
pub fn parse(source: &str) -> Result<ParseOutput, ParseFailure> {
    parse_with_options(source, ParseOptions::default())
}

pub fn parse_with_options(
    source: &str,
    options: ParseOptions<'_>,
) -> Result<ParseOutput, ParseFailure> {
    parse_with_spans(source, options, SpanTable::new())
}

/// Parse file `file` of a multi-file module. Its span ids are unique across the
/// module and name `file`, so one checker can resolve every file's spans.
/// `file` must be below [`SpanId::FILES_MAX`]; callers bound the file count.
pub fn parse_file(
    source: &str,
    file: u32,
    options: ParseOptions<'_>,
) -> Result<ParseOutput, ParseFailure> {
    parse_with_spans(source, options, SpanTable::for_file(file))
}

fn parse_with_spans(
    source: &str,
    options: ParseOptions<'_>,
    spans: SpanTable,
) -> Result<ParseOutput, ParseFailure> {
    options.limits.validate()?;
    let mut cursor = Cursor::lex_in(source, options, spans)?;
    let items = cursor.parse_items()?;
    if cursor.spans.overflowed() {
        return Err(cursor.span_limit_failure());
    }
    Ok(ParseOutput {
        file: SourceFile {
            text: source.into(),
            span_table: cursor.spans,
            items,
            comments: cursor.comments,
        },
        diagnostics: cursor.diagnostics,
    })
}

/// Strict adapter for file `file` of a multi-file module; see [`parse_file`].
pub fn parse_strict_file(source: &str, file: u32) -> Result<SourceFile, Vec<Diagnostic>> {
    parse_file(source, file, ParseOptions::default())
        .map_err(|failure| vec![failure.diagnostic()])?
        .into_result()
}

/// Compiler-style adapter over the same recovering parser, never a separate grammar.
pub fn parse_strict(source: &str) -> Result<SourceFile, Vec<Diagnostic>> {
    parse(source)
        .map_err(|failure| vec![failure.diagnostic()])?
        .into_result()
}
