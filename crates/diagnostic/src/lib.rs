//! Unified diagnostics for the ginc compiler.
//!
//! This module provides a single `Symptom` type that encompasses all
//! flaw/help/info types produced by the compiler, with support for:
//! - Source spans (using 0..0 for flaws without a location)
//! - Severity levels (Flaw, Hint, Info)
//! - Error codes (`{stage}-{name}` format, e.g. `lex-unexpected-char`)

mod category;
mod source;
pub use category::Category;
pub use source::*;
pub use span::{Span, SpanId, SpanTable, Spanned};

pub trait SymptomLike: Sized {
    fn into_symptom(self, span_id: SpanId) -> Symptom;
}

#[derive(Debug, Clone)]
pub struct Symptom {
    pub code: &'static str,
    pub message: String,
    pub help: Option<String>,
    pub span_id: SpanId,
    pub category: Category,
}

impl Symptom {
    pub fn error_code(&self) -> &'static str {
        self.code
    }

    /// Pretty-print this symptom using ariadne with source context.
    pub fn print(&self, span_table: &SpanTable, source: &str, filename: &str) {
        use ariadne::{Label, Report, ReportKind, Source};
        use std::ops::Range;

        let span = span_table.get(self.span_id);
        let start = span.start;
        let end = span.end;

        // Clamp span to source bounds
        let len = source.len();
        let start = start.min(len);
        let end = end.max(start).min(len);

        let kind = ReportKind::Custom(self.category.as_str(), self.category.color());

        let span: Range<usize> = start..end;

        let msg = format!("[{}] {}", self.code, self.message);
        let mut builder = Report::build(kind, (filename, span.clone())).with_message(msg);

        let is_unclosed_string = self.code == "lex-unclosed-string";

        if is_unclosed_string && start < end {
            let mut display_source = source.to_string();
            display_source.insert(end, '\'');

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

            if let Some(help) = &self.help {
                label = label.with_message(help);
            }

            builder = builder.with_label(label);
        } else if start > 0 {
            let back = (start - 1)..start;
            let mut label = Label::new((filename, back))
                .with_color(self.category.color())
                .with_message("here");
            if let Some(help) = &self.help {
                label = label.with_message(help);
            }
            builder = builder.with_label(label);
        } else if let Some(help) = &self.help {
            builder = builder.with_note(help);
        }

        let report = builder.finish();
        report
            .eprint((filename, Source::from(source)))
            .unwrap_or_else(|e| eprintln!("Failed to print diagnostic: {e}"));
    }
}
