use clap::ValueEnum;
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

#[derive(Debug, Default)]
pub struct CheckOptions {
    pub message_format: MessageFormat,
    pub max_errors: Option<usize>,
}

pub(crate) struct DiagnosticReporter {
    options: CheckOptions,
    root: PathBuf,
    seen: HashSet<(PathBuf, Span, Category, String, String)>,
    errors: usize,
    warnings: usize,
    omitted: usize,
}

impl DiagnosticReporter {
    pub(crate) fn new(input: &Path, options: CheckOptions) -> Self {
        let root = flask::FlaskConfig::find_package_config(input)
            .map(|(_, root)| root)
            .unwrap_or_else(|| {
                if input.is_dir() {
                    input.to_path_buf()
                } else {
                    input.parent().unwrap_or(Path::new(".")).to_path_buf()
                }
            })
            .normalize_diagnostic_path();
        Self {
            options,
            root,
            seen: HashSet::new(),
            errors: 0,
            warnings: 0,
            omitted: 0,
        }
    }

    pub(crate) fn emit(&mut self, path: &Path, source: &str, diagnostics: &[Diagnostic]) -> bool {
        let path = path.normalize_diagnostic_path();
        let filename = path.diagnostic_report_path_from_cwd();
        let dependency = flask::FlaskConfig::find_package_config(&path)
            .map(|(_, root)| root.normalize_diagnostic_path() != self.root)
            .unwrap_or_else(|| !path.starts_with(&self.root));
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
            if !self.seen.insert((
                path.clone(),
                d.span,
                d.category,
                d.code.slug().to_owned(),
                d.message.clone(),
            )) {
                continue;
            }
            match d.category {
                Category::Flaw => self.errors += 1,
                Category::Warn => self.warnings += 1,
                Category::Help => {}
            }
            if d.category == Category::Flaw
                && self
                    .options
                    .max_errors
                    .is_some_and(|limit| self.errors > limit)
            {
                self.omitted += 1;
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
                    let (line, column) = position(source, d.span.start());
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
                        .related
                        .iter()
                        .map(|r| json!({"range": range(source, r.span), "label": r.label}))
                        .collect();
                    println!(
                        "{}",
                        json!({
                            "reason": "diagnostic", "severity": severity, "code": d.code.slug(),
                            "file": path, "range": range(source, d.span), "message": d.message,
                            "help": d.help, "label": d.help_on_span, "related": related,
                            "args": d.args, "dependency": dependency
                        })
                    );
                }
            }
        }
        failed
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
            eprintln!("begin check: {message}");
        }
    }

    pub(crate) fn finish(&self, success: bool) {
        if self.options.message_format == MessageFormat::Json {
            println!(
                "{}",
                json!({"reason": "check-finished", "success": success, "errors": self.errors, "warnings": self.warnings, "omitted_errors": self.omitted})
            );
        } else {
            let omitted = if self.omitted == 0 {
                String::new()
            } else {
                format!(" ({} errors omitted)", self.omitted)
            };
            eprintln!(
                "Check {}: {} errors, {} warnings{omitted}",
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

fn position(source: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = &source[..offset];
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

fn range(source: &str, span: Span) -> Value {
    let (start_line, start_column) = position(source, span.start());
    let (end_line, end_column) = position(source, span.end());
    json!({"byte_start": span.start(), "byte_end": span.end(),
        "start": {"line": start_line, "column": start_column},
        "end": {"line": end_line, "column": end_column}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_identity_keeps_distinct_files_spans_and_messages() {
        let mut reporter = DiagnosticReporter::new(
            Path::new("/virtual"),
            CheckOptions {
                max_errors: Some(0),
                ..Default::default()
            },
        );
        let d = Diagnostic::new("type-unknown-symbol", "unknown symbol").at_span(Span::new(1, 2));
        assert!(reporter.emit(
            Path::new("/virtual/a.gin"),
            "abc",
            &[d.clone(), d.clone().with_help("additional help")]
        ));
        reporter.emit(Path::new("/virtual/b.gin"), "abc", &[d.clone()]);
        reporter.emit(
            Path::new("/virtual/a.gin"),
            "abc",
            &[
                d.at_span(Span::new(2, 3)),
                Diagnostic::new("type-unknown-symbol", "another symbol").at_span(Span::new(1, 2)),
            ],
        );
        assert_eq!(reporter.errors, 4);
        assert_eq!(reporter.omitted, 4);
    }

    #[test]
    fn positions_use_one_based_unicode_columns_and_byte_offsets() {
        assert_eq!(position("é\nλ missing", 5), (2, 2));
        let range = range("é\nλ missing", Span::new(6, 13));
        assert_eq!(range["start"]["column"], 3);
        assert_eq!(range["start"]["line"], 2);
        assert_eq!(range["byte_start"], 6);
    }
}
