use clap::ValueEnum;
#[cfg(test)]
use diagnostic::code::SymbolCode;
use diagnostic::{Category, Diagnostic, DiagnosticPathExt, Span};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum MessageFormat {
    #[default]
    Human,
    Short,
    /// JSON lines with UTF-8 byte ranges and one-based line and Unicode character columns.
    Json,
}

/// Whose warnings are reported. Errors are always reported, wherever they are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum WarningScope {
    /// Warnings of the package being built only.
    #[default]
    Own,
    /// Also the warnings of its dependencies.
    Deps,
}

#[derive(Debug, Default)]
pub struct CheckOptions {
    pub message_format: MessageFormat,
    pub max_errors: Option<usize>,
}

pub(crate) struct DiagnosticReporter {
    options: CheckOptions,
    seen: HashSet<(PathBuf, Option<Span>, Category, String, String)>,
    errors: usize,
    warnings: usize,
    omitted: usize,
    hidden_warnings: usize,
    in_dependency: bool,
}

impl DiagnosticReporter {
    pub(crate) fn new(options: CheckOptions) -> Self {
        Self {
            options,
            seen: HashSet::new(),
            errors: 0,
            warnings: 0,
            omitted: 0,
            hidden_warnings: 0,
            in_dependency: false,
        }
    }

    /// Package loading supplies the origin rather than rediscovering it per diagnostic.
    pub(crate) fn set_dependency(&mut self, in_dependency: bool) {
        self.in_dependency = in_dependency;
    }

    /// Count a warning from a dependency that is not shown.
    pub(crate) fn hide_warning(&mut self) {
        self.hidden_warnings += 1;
    }

    pub(crate) fn emit(&mut self, path: &Path, source: &str, diagnostics: &[Diagnostic]) -> bool {
        let path = path.normalize_diagnostic_path();
        let filename = path.diagnostic_report_path_from_cwd();
        let dependency = self.in_dependency;
        let mut failed = false;
        let mut diagnostics: Vec<_> = diagnostics.iter().collect();
        diagnostics.sort_by(|a, b| {
            (a.span.start(), a.span.end(), a.code.slug(), &a.message).cmp(&(
                b.span.start(),
                b.span.end(),
                b.code.slug(),
                &b.message,
            ))
        });
        for d in diagnostics {
            failed |= d.category == Category::Flaw;
            if !self.record(&path, Some(d.span), d.category, d.code, &d.message) {
                continue;
            }
            let severity = d.category.as_str();
            match self.options.message_format {
                MessageFormat::Human => {
                    if dependency {
                        eprintln!("dependency: {filename}");
                    }
                    d.print(source, &filename);
                }
                MessageFormat::Short => {
                    let (line, column) = diagnostic::source_position(source, d.span.start());
                    let origin = if dependency { " [dependency]" } else { "" };
                    let help = d
                        .help
                        .as_deref()
                        .or(d.help_on_span.as_deref())
                        .map(|h| format!("; help: {h}"))
                        .unwrap_or_default();
                    eprintln!(
                        "{}:{line}:{column}: {severity}[{}]{origin}: {}{}",
                        filename,
                        d.code.slug(),
                        single_line(&d.message),
                        single_line(&help)
                    );
                }
                MessageFormat::Json => {
                    let related: Vec<_> = d
                        .related()
                        .iter()
                        .map(|r| json!({"range": range(source, r.span), "label": r.label}))
                        .collect();
                    println!(
                        "{}",
                        json!({
                            "reason": "diagnostic", "severity": severity, "code": d.code.slug().as_str(),
                            "file": path, "range": range(source, d.span), "message": d.message,
                            "help": d.help, "label": d.help_on_span, "related": related,
                            "args": d.args(), "dependency": dependency
                        })
                    );
                }
            }
        }
        failed
    }

    fn record(
        &mut self,
        path: &Path,
        span: Option<Span>,
        category: Category,
        code: diagnostic::DiagnosticCode,
        message: &str,
    ) -> bool {
        if !self.seen.insert((
            path.to_path_buf(),
            span,
            category,
            code.to_string(),
            message.to_owned(),
        )) {
            return false;
        }
        match category {
            Category::Flaw => self.errors += 1,
            Category::Warn => self.warnings += 1,
            Category::Help => {}
        }
        if category == Category::Flaw
            && self
                .options
                .max_errors
                .is_some_and(|limit| self.errors > limit)
        {
            self.omitted += 1;
            return false;
        }
        true
    }

    /// Report a standalone failure without pretending byte zero is its location.
    pub(crate) fn emit_unlocated(
        &mut self,
        path: &Path,
        code: diagnostic::DiagnosticCode,
        message: &str,
    ) {
        let path = path.normalize_diagnostic_path();
        if !self.record(&path, None, Category::Flaw, code, message) {
            return;
        }
        match self.options.message_format {
            MessageFormat::Json => {
                println!(
                    "{}",
                    unlocated_json(&path, code, message, self.in_dependency)
                );
            }
            MessageFormat::Human | MessageFormat::Short => {
                let filename = path.diagnostic_report_path_from_cwd();
                let origin = if self.in_dependency {
                    " [dependency]"
                } else {
                    ""
                };
                eprintln!(
                    "{filename}: flaw[{}]{origin}: {}",
                    code.slug(),
                    single_line(message)
                );
            }
        }
    }

    pub(crate) fn failure(&mut self, message: &str) {
        self.errors += 1;
        if self
            .options
            .max_errors
            .is_some_and(|limit| self.errors > limit)
        {
            self.omitted += 1;
            return;
        }
        if self.options.message_format == MessageFormat::Json {
            println!(
                "{}",
                json!({"reason": "diagnostic", "severity": "flaw", "code": "check-failed", "file": null, "range": null, "message": message, "help": null, "related": [], "dependency": false})
            );
        } else {
            eprintln!("error: {message}");
        }
    }

    pub(crate) fn finish(&self, success: bool) {
        if self.options.message_format == MessageFormat::Json {
            println!(
                "{}",
                json!({"reason": "check-finished", "success": success, "errors": self.errors, "warnings": self.warnings, "omitted_errors": self.omitted, "hidden_warnings": self.hidden_warnings})
            );
        } else {
            let omitted = if self.omitted == 0 {
                String::new()
            } else {
                format!(" ({} errors omitted)", self.omitted)
            };
            let hidden = if self.hidden_warnings == 0 {
                String::new()
            } else {
                format!(
                    " ({} dependency warnings hidden; --warnings deps shows them)",
                    self.hidden_warnings
                )
            };
            eprintln!(
                "Check {}: {} errors, {} warnings{omitted}{hidden}",
                if success { "passed" } else { "failed" },
                self.errors,
                self.warnings
            );
        }
    }
}

fn single_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

fn unlocated_json(
    path: &Path,
    code: diagnostic::DiagnosticCode,
    message: &str,
    dependency: bool,
) -> Value {
    json!({
        "reason": "diagnostic", "severity": "flaw", "code": code.slug(),
        "file": path, "range": null, "message": message,
        "help": null, "label": null, "related": [], "args": [], "dependency": dependency
    })
}

fn range(source: &str, span: Span) -> Value {
    let (start_line, start_column) = diagnostic::source_position(source, span.start());
    let (end_line, end_column) = diagnostic::source_position(source, span.end());
    json!({"byte_start": span.start(), "byte_end": span.end(),
        "start": {"line": start_line, "column": start_column},
        "end": {"line": end_line, "column": end_column}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_identity_keeps_distinct_files_spans_and_messages() {
        let mut reporter = DiagnosticReporter::new(CheckOptions {
            max_errors: Some(0),
            ..Default::default()
        });
        let d = Diagnostic::new(SymbolCode::Unknown, "unknown symbol").at_span(Span::new(1, 2));
        assert!(reporter.emit(
            Path::new("/virtual/a.gin"),
            "abc",
            &[d.clone(), d.clone().with_help("additional help")]
        ));
        reporter.emit(Path::new("/virtual/b.gin"), "abc", std::slice::from_ref(&d));
        reporter.emit(
            Path::new("/virtual/a.gin"),
            "abc",
            &[
                d.at_span(Span::new(2, 3)),
                Diagnostic::new(SymbolCode::Unknown, "another symbol").at_span(Span::new(1, 2)),
            ],
        );
        assert_eq!(reporter.errors, 4);
        assert_eq!(reporter.omitted, 4);
    }

    #[test]
    fn positions_use_one_based_unicode_columns_and_byte_offsets() {
        assert_eq!(diagnostic::source_position("é\nλ missing", 5), (2, 2));
        let range = range("é\nλ missing", Span::new(6, 13));
        assert_eq!(range["start"]["column"], 3);
        assert_eq!(range["start"]["line"], 2);
        assert_eq!(range["byte_start"], 6);
    }

    #[test]
    fn standalone_unlocated_errors_have_no_fabricated_range() {
        let code = diagnostic::code::SourceCheckCode::Budget.into();
        for dependency in [false, true] {
            let value = unlocated_json(
                Path::new("/virtual/source.gin"),
                code,
                "budget exceeded",
                dependency,
            );
            assert_eq!(value["range"], Value::Null);
            assert_eq!(value["code"], "source-check-budget");
            assert_eq!(value["dependency"], dependency);
        }
    }

    #[test]
    fn standalone_unlocated_errors_share_deduplication_and_limits() {
        let mut reporter = DiagnosticReporter::new(CheckOptions {
            message_format: MessageFormat::Short,
            max_errors: Some(0),
        });
        let path = Path::new("/virtual/source.gin");
        let code = diagnostic::code::SourceCheckCode::Budget;
        reporter.emit_unlocated(path, code.into(), "budget exceeded");
        reporter.emit_unlocated(path, code.into(), "budget exceeded");
        reporter.emit(
            path,
            "x",
            &[Diagnostic::new(code, "budget exceeded").at_span(Span::new(0, 0))],
        );
        assert_eq!(reporter.errors, 2);
        assert_eq!(reporter.omitted, 2);
    }
}
