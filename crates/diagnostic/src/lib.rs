//! Unified diagnostics for the ginc compiler.
//!
//! This module provides a single `Diagnostic` type that encompasses all
//! flaw/help/info types produced by the compiler, with support for:
//! - Source spans (using 0..0 for diagnostics without a location)
//! - Severity levels (Flaw, Hint, Info)
//! - Error codes (`{stage}-{name}` format, e.g. `lex-unexpected-character`)
//!
//! Diagnostics are constructed via the builder pattern, e.g.:
//! ```ignore
//! Diagnostic::new(SymbolCode::Unknown, "unknown symbol `Foo`")
//!     .with_arg("name", "Foo")
//!     .with_help("add an import for `Foo`")
//!     .at_span_id(span_id, &span_table)
//! ```

#[cfg(test)]
extern crate self as diagnostic;

pub use span::{Span, SpanId, SpanTable, Spanned};
mod category;
pub mod code;
use crate::code::LexCode;
pub use category::Category;
pub use code::DiagnosticCode;
use std::path::{Component, Path, PathBuf};

/// Extension trait providing diagnostic path utilities on [`Path`].
pub trait DiagnosticPathExt {
    /// Return a stable, absolute form of a path for diagnostic identity.
    ///
    /// Existing paths are canonicalized. Non-existing paths are made absolute against
    /// the process current directory and normalized lexically, so CLI and LSP callers
    /// can use the same keys even before a file has been written to disk.
    #[must_use]
    fn normalize_diagnostic_path(&self) -> PathBuf;

    /// Return the display label used by terminal diagnostics.
    ///
    /// The label is relative to `base` when possible, otherwise it falls back to the
    /// normalized absolute path. This is intentionally shared by tools so path labels
    /// do not drift between diagnostic producers.
    #[must_use]
    fn diagnostic_report_path(&self, base: &Path) -> String;

    /// Return the display label used by terminal diagnostics, relative to the
    /// process current directory when possible.
    #[must_use]
    fn diagnostic_report_path_from_cwd(&self) -> String;

    /// Lexically normalize a path by resolving `.` and `..` components without
    /// touching the filesystem.
    #[must_use]
    fn lexical_normalize(&self) -> PathBuf;
}

impl DiagnosticPathExt for Path {
    fn normalize_diagnostic_path(&self) -> PathBuf {
        if let Ok(canonical) = self.canonicalize() {
            return canonical;
        }

        let absolute = if self.is_absolute() {
            self.to_path_buf()
        } else if let Ok(cwd) = std::env::current_dir() {
            cwd.join(self)
        } else {
            self.to_path_buf()
        };

        absolute.lexical_normalize()
    }

    fn diagnostic_report_path(&self, base: &Path) -> String {
        let path = self.normalize_diagnostic_path();
        let base = base.normalize_diagnostic_path();
        path.strip_prefix(&base)
            .map(|p| {
                let s = p.display().to_string();
                if s.is_empty() { ".".to_string() } else { s }
            })
            .unwrap_or_else(|_| path.display().to_string())
    }

    fn diagnostic_report_path_from_cwd(&self) -> String {
        std::env::current_dir()
            .map(|cwd| self.diagnostic_report_path(&cwd))
            .unwrap_or_else(|_| self.normalize_diagnostic_path().display().to_string())
    }

    fn lexical_normalize(&self) -> PathBuf {
        let mut out = PathBuf::new();
        for component in self.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                    out.push(component.as_os_str());
                }
            }
        }
        out
    }
}

/// A secondary span label attached to a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelatedSpan {
    pub span: Span,
    pub label: String,
}

/// 1-based line and column (in Unicode scalars) of `byte` in `source`.
///
/// The offset is clamped to the nearest preceding char boundary. This is the
/// CLI/JSON reporting convention; it is NOT the 0-based UTF-16 convention used
/// by LSP position conversions.
#[must_use]
pub fn source_position(source: &str, byte: usize) -> (usize, usize) {
    let mut byte = byte.min(source.len());
    while !source.is_char_boundary(byte) {
        byte -= 1;
    }
    let prefix = &source[..byte];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count()
        + 1;
    (line, column)
}

/// Structured secondary data attached to a [`Diagnostic`].
///
/// Kept behind a box on [`Diagnostic`] so the diagnostic stays below clippy's
/// `result_large_err` threshold while retaining its rich error/code payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct DiagnosticDetails {
    /// Structured key-value args for IDE quickfix data extraction (e.g. symbol name).
    /// Only populated when a quickfix is applicable.
    pub args: Vec<(String, String)>,
    pub related: Vec<RelatedSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub message: String,
    /// Shown on the source underline in terminal reports. When set with [`Self::help`], the
    /// latter is rendered as a separate `help: …` line (LSP and ariadne).
    pub help_on_span: Option<String>,
    pub help: Option<String>,
    /// Byte range in the source file this diagnostic applies to.
    pub span: Span,
    pub category: Category,
    details: Option<Box<DiagnosticDetails>>,
}

impl Diagnostic {
    /// Create a new diagnostic with the given code and human-readable message.
    ///
    /// The message should be a self-contained description. Use [`with_arg`](Self::with_arg)
    /// to attach structured data for IDE quickfix extraction.
    pub fn new(code: impl Into<DiagnosticCode>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            help_on_span: None,
            help: None,
            span: Span::new(0, 0),
            category: Category::Flaw,
            details: None,
        }
    }

    fn details_mut(&mut self) -> &mut DiagnosticDetails {
        self.details
            .get_or_insert_with(|| Box::new(DiagnosticDetails::default()))
    }

    /// Structured key-value args for IDE quickfix extraction.
    #[must_use]
    pub fn args(&self) -> &[(String, String)] {
        match &self.details {
            Some(details) => &details.args,
            None => &[],
        }
    }

    /// Secondary spans attached to this diagnostic.
    #[must_use]
    pub fn related(&self) -> &[RelatedSpan] {
        match &self.details {
            Some(details) => &details.related,
            None => &[],
        }
    }

    pub fn related_mut(&mut self) -> &mut Vec<RelatedSpan> {
        &mut self.details_mut().related
    }

    /// Attach a structured key-value pair for IDE quickfix extraction.
    ///
    /// Example: `Diagnostic::new(SymbolCode::Unknown, …).with_arg("name", "Foo")`
    /// allows the LSP handler to do `diag.arg("name") → Some("Foo")` in O(1).
    pub fn with_arg(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details_mut().args.push((key.into(), value.into()));
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_help_on_span(mut self, text: impl Into<String>) -> Self {
        self.help_on_span = Some(text.into());
        self
    }

    /// Set the source byte range this diagnostic applies to.
    pub fn at_span(mut self, span: Span) -> Self {
        self.span = span;
        self
    }

    /// Resolve a [`SpanId`] through a table and set the source span.
    pub fn at_span_id(self, span_id: SpanId, table: &SpanTable) -> Self {
        self.at_span(table.get(span_id))
    }

    pub fn with_category(mut self, category: Category) -> Self {
        self.category = category;
        self
    }

    /// Retrieve a structured arg by key (used by IDE quickfix code).
    #[must_use]
    pub fn arg(&self, key: &str) -> Option<&str> {
        self.args()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[must_use]
    pub fn error_code(&self) -> String {
        self.code.slug()
    }

    /// Pretty-print this diagnostic using ariadne with source context.
    pub fn print(&self, source: &str, filename: &str) {
        use ariadne::{Label, Report, ReportKind, Source};
        use std::io::IsTerminal;
        use std::ops::Range;

        let start = self.span.start();
        let end = self.span.end();

        // Clamp span to source bounds
        let len = source.len();
        let start = start.min(len);
        let end = end.max(start).min(len);

        let kind = ReportKind::Custom(self.category.as_str(), self.category.color());

        let span: Range<usize> = start..end;

        let msg = format!("[{}] {}", self.error_code(), self.message);
        let mut builder = Report::build(kind, (filename, span.clone()))
            .with_config(
                ariadne::Config::default()
                    .with_color(std::io::stderr().is_terminal())
                    .with_index_type(ariadne::IndexType::Byte),
            )
            .with_message(msg);

        // Let custom rendering handle this if needed (e.g. unclosed strings).
        if self.render_custom(source, filename) {
            return;
        }

        let color = self.category.color();

        let underline = self
            .help_on_span
            .as_deref()
            .or(self.help.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if start < end {
            let mut primary = Label::new((filename, span.clone())).with_color(color);
            if let Some(m) = underline {
                primary = primary.with_message(m);
            }
            builder = builder.with_label(primary);
        } else if start > 0 {
            let back = (start - 1)..start;
            let mut label = Label::new((filename, back))
                .with_color(color)
                .with_message("here");
            if let Some(m) = underline {
                label = label.with_message(m);
            }
            builder = builder.with_label(label);
        } else if let Some(help) = self
            .help
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            builder = builder.with_note(help);
        }

        if (start < end || start > 0)
            && self.help_on_span.is_some()
            && let Some(h) = self
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

        // Render related spans as secondary labels.
        for related in self.related() {
            let rstart = related.span.start();
            let rend = related.span.end();
            let rstart = rstart.min(len);
            let rend = rend.max(rstart).min(len);
            if rstart < rend {
                let label =
                    Label::new((filename, rstart..rend)).with_message(related.label.as_str());
                builder = builder.with_label(label);
            }
        }

        let report = builder.finish();
        report
            .eprint((filename, Source::from(source)))
            .unwrap_or_else(|e| eprintln!("Failed to print diagnostic: {e}"));
    }

    /// Render this diagnostic with custom ariadne output.
    /// Returns `true` if the diagnostic was fully handled (the caller should skip
    /// default rendering).
    pub fn render_custom(&self, source: &str, filename: &str) -> bool {
        match self.code {
            DiagnosticCode::Lex(LexCode::UnclosedString) => {
                self.render_unclosed_string(source, filename)
            }
            _ => false,
        }
    }

    /// Custom ariadne rendering for unclosed string literals: inserts a closing quote
    /// suggestion into the displayed source.
    fn render_unclosed_string(&self, source: &str, filename: &str) -> bool {
        use ariadne::{Label, Report, ReportKind, Source};
        use std::io::IsTerminal;
        use std::ops::Range;

        let len = source.len();
        let start = self.span.start();
        let end = self.span.end();
        let start = start.min(len);
        let end = end.max(start).min(len);

        if start >= end {
            return false;
        }

        let mut display_source = source.to_string();
        display_source.insert(end, '\'');

        let quote_span = end..end + 1;
        let kind = ReportKind::Custom(Category::Flaw.as_str(), Category::Flaw.color());
        let span_range: Range<usize> = start..end;
        let msg = format!("[{}] {}", self.error_code(), self.message);
        let builder = Report::build(kind, (filename, span_range))
            .with_config(
                ariadne::Config::default()
                    .with_color(std::io::stderr().is_terminal())
                    .with_index_type(ariadne::IndexType::Byte),
            )
            .with_message(&msg)
            .with_label(
                Label::new((filename, quote_span))
                    .with_color(Category::Flaw.color())
                    .with_message("add single quote here"),
            );

        let report = builder.finish();
        report
            .eprint((filename, Source::from(display_source)))
            .unwrap_or_else(|e| eprintln!("Failed to print diagnostic: {e}"));
        true
    }
}
#[cfg(test)]
#[path = "../tests/lib_tests.rs"]
mod tests;
