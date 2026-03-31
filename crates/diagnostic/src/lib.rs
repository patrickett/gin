//! Unified diagnostics for the ginc compiler.
//!
//! This module provides a single `Symptom` type that encompasses all
//! flaw/help/info types produced by the compiler, with support for:
//! - Source spans (using 0..0 for flaws without a location)
//! - Severity levels (Flaw, Hint, Info)
//! - Error codes (E001-E499, similar to eslint)
//! - Help text for suggestions

mod category;
mod source;

pub use category::Category;
use chumsky::span::SimpleSpan;
pub use source::*;

#[salsa::accumulator]
pub struct Symptom {
    pub source: SymptomSource,
    pub span: SimpleSpan,
    pub category: Category,
}

impl Symptom {
    pub fn error_code(&self) -> String {
        let category = self.category.as_char();
        let prefix = self.source.prefix();
        let id = self.source.id();
        format!("{category}{prefix}{id:03}")
    }

    pub fn message(&self) -> String {
        self.source.message()
    }

    pub fn help(&self) -> Option<String> {
        self.source.help()
    }

    /// Pretty-print this symptom using ariadne with source context.
    pub fn print(&self, source: &str, filename: &str) {
        use ariadne::{Label, Report, ReportKind, Source};
        use std::ops::Range;

        let start = self.span.start;
        let end = self.span.end;

        // Clamp span to source bounds
        let len = source.len();
        let start = start.min(len);
        let end = end.max(start).min(len);

        let kind = ReportKind::Custom(self.category.as_str(), self.category.color());

        let span: Range<usize> = start..end;

        let msg = format!("[{}] {}", self.error_code(), self.message());
        let mut builder = Report::build(kind, (filename, span.clone())).with_message(msg);

        // Check if this is an unclosed string error for special rendering
        let is_unclosed_string =
            matches!(&self.source, SymptomSource::Lex(LexSymptom::UnclosedString));

        if is_unclosed_string && start < end {
            // For unclosed strings, insert a red quote at the end position for display
            let mut display_source = source.to_string();
            // TODO: if end < display_source.len() {} to avoid panic below
            display_source.insert(end, '\'');

            // Use a 1-char span at the end position (the inserted quote)
            let quote_span = end..end + 1;

            let label = Label::new((filename, quote_span))
                .with_color(self.category.color())
                .with_message("add single quote here");
            builder = builder.with_label(label);

            let report = builder.finish();
            report
                .eprint((filename, Source::from(display_source)))
                .unwrap_or_else(|e| eprintln!("Failed to print diagnostic: {e}"));
            return;
        }

        if start < end {
            let mut label = Label::new((filename, span)).with_color(self.category.color());

            if let Some(help) = self.help() {
                label = label.with_message(help);
            }

            builder = builder.with_label(label);
        } else if let Some(help) = self.help() {
            builder = builder.with_note(help);
        }

        let report = builder.finish();
        report
            // TODO: write reports to buffer then put them on stdout
            // just for speed improvement so we are not waiting on std io to finish
            .eprint((filename, Source::from(source)))
            .unwrap_or_else(|e| eprintln!("Failed to print diagnostic: {e}"));
    }
}

pub enum SymptomSource {
    Lex(LexSymptom),
    Parse(ParseSymptom),
    CodeGen(CodegenSymptom),
    Io(IoSymptom),
    Type(TypeSymptom),
}

pub trait SymptomDetail {
    fn id(&self) -> u8;
    fn message(&self) -> String;
    fn help(&self) -> Option<String>;
}

impl SymptomSource {
    fn prefix(&self) -> char {
        match self {
            SymptomSource::Lex(_) => 'L',
            SymptomSource::Parse(_) => 'P',
            SymptomSource::CodeGen(_) => 'C',
            SymptomSource::Io(_) => 'I',
            SymptomSource::Type(_) => 'T',
        }
    }
}

impl SymptomDetail for SymptomSource {
    fn id(&self) -> u8 {
        match self {
            SymptomSource::Lex(s) => s.id(),
            SymptomSource::Parse(s) => s.id(),
            SymptomSource::CodeGen(s) => s.id(),
            SymptomSource::Io(s) => s.id(),
            SymptomSource::Type(s) => s.id(),
        }
    }

    fn message(&self) -> String {
        match self {
            SymptomSource::Lex(s) => s.message(),
            SymptomSource::Parse(s) => s.message(),
            SymptomSource::CodeGen(s) => s.message(),
            SymptomSource::Io(s) => s.message(),
            SymptomSource::Type(s) => s.message(),
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            SymptomSource::Lex(s) => s.help(),
            SymptomSource::Parse(s) => s.help(),
            SymptomSource::CodeGen(s) => s.help(),
            SymptomSource::Io(s) => s.help(),
            SymptomSource::Type(s) => s.help(),
        }
    }
}
