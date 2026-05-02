//! Unified diagnostics for the ginc compiler.
//!
//! This module provides a single `Diagnostic` type that encompasses all
//! flaw/help/info types produced by the compiler, with support for:
//! - Source spans (using 0..0 for diagnostics without a location)
//! - Severity levels (Flaw, Hint, Info)
//! - Error codes (`{stage}-{name}` format, e.g. `lex-unexpected-character`)

mod category;
mod code;
mod domain;
pub use category::Category;
pub use code::*;
pub use domain::*;
pub use span::{Span, SpanId, SpanTable, Spanned};

/// A secondary span label attached to a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedSpan {
    pub span_id: SpanId,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub message: String,
    /// Shown on the source underline in terminal reports. When set with [`Self::help`], the
    /// latter is rendered as a separate `help: …` line (LSP and ariadne).
    pub help_on_span: Option<String>,
    pub help: Option<String>,
    pub span_id: SpanId,
    pub category: Category,
    pub related: Vec<RelatedSpan>,
}

/// Trait for domain-specific diagnostic types that know how to describe themselves.
///
/// Implementors provide their message, optional `help` / `help_on_span`, and category.
/// The `into_diagnostic` default impl wraps them into a [`Diagnostic`].
pub trait DiagnosticLike: Sized {
    /// The primary message for this diagnostic.
    fn message(&self) -> String;

    /// Optional help text suggesting how to fix the issue.
    fn help(&self) -> Option<String> {
        None
    }

    /// Optional text on the span underline in terminal output (see [`Diagnostic::help_on_span`]).
    fn help_on_span(&self) -> Option<String> {
        None
    }

    /// The severity category. Defaults to `Category::Flaw`.
    fn category(&self) -> Category {
        Category::Flaw
    }

    /// Convert into a full `Diagnostic` anchored at the given span.
    fn into_diagnostic(self, span_id: SpanId) -> Diagnostic
    where
        Self: Into<DiagnosticCode>,
    {
        let message = self.message();
        let help_on_span = self.help_on_span();
        let help = self.help();
        let category = self.category();
        let code: DiagnosticCode = self.into();
        Diagnostic {
            message,
            help_on_span,
            help,
            category,
            code,
            span_id,
            related: Vec::new(),
        }
    }
}

impl Diagnostic {
    pub fn error_code(&self) -> &str {
        self.code.slug()
    }

    /// Pretty-print this diagnostic using ariadne with source context.
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

        let msg = format!("[{}] {}", self.error_code(), self.message);
        let mut builder = Report::build(kind, (filename, span.clone())).with_message(msg);

        // Let the domain type do custom rendering if needed.
        if self.code.render_custom(self, span_table, source, filename) {
            return;
        }

        let color = self.category.color();

        if start < end {
            let underline = self
                .help_on_span
                .as_deref()
                .or(self.help.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let mut primary = Label::new((filename, span.clone())).with_color(color);
            if let Some(m) = underline {
                primary = primary.with_message(m);
            }
            builder = builder.with_label(primary);

            if self.help_on_span.is_some() {
                if let Some(h) = self
                    .help
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let point = start..start;
                    let secondary = Label::new((filename, point))
                        .with_color(color)
                        .with_message(format!("help: {h}"))
                        .with_order(1);
                    builder = builder.with_label(secondary);
                }
            }
        } else if start > 0 {
            let back = (start - 1)..start;
            let underline = self
                .help_on_span
                .as_deref()
                .or(self.help.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let mut label = Label::new((filename, back))
                .with_color(color)
                .with_message("here");
            if let Some(m) = underline {
                label = label.with_message(m);
            }
            builder = builder.with_label(label);

            if self.help_on_span.is_some() {
                if let Some(h) = self
                    .help
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let point = start..start;
                    let secondary = Label::new((filename, point))
                        .with_color(color)
                        .with_message(format!("help: {h}"))
                        .with_order(1);
                    builder = builder.with_label(secondary);
                }
            }
        } else if let Some(help) = self
            .help
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            builder = builder.with_note(help);
        }

        // Render related spans as secondary labels.
        for related in &self.related {
            let rspan = span_table.get(related.span_id);
            let rstart = rspan.start.min(len);
            let rend = rspan.end.max(rstart).min(len);
            if rstart < rend {
                let label = Label::new((filename, rstart..rend))
                    .with_message(related.label.as_str());
                builder = builder.with_label(label);
            }
        }

        let report = builder.finish();
        report
            .eprint((filename, Source::from(source)))
            .unwrap_or_else(|e| eprintln!("Failed to print diagnostic: {e}"));
    }
}
