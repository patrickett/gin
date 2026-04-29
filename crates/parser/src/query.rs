//! Pure parsing functions — no Salsa or database dependency.
//!
//! This module exposes pure functions that parse source text and extract
//! information from ASTs without relying on incremental compilation
//! infrastructure.

use ast::span::{HasSpanId, SpanId, SpanTable};
use diagnostic::LexSymptom;
use diagnostic::parse::ParseSymptom;
use diagnostic::{Diagnostic, DiagnosticLike};
use lexer::{Lexer, Token};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::expr;
use ast::{BindValue, Expr, FileAst, FnCall, ImportSource};
use flask::FlaskConfig;

// TODO: change ParseOutput to a Vec<Diagnostic> or some vec of enum so we dont need the different
// fields
/// Full output from parsing source text, including all diagnostic info.
#[derive(Clone, PartialEq, Eq)]
pub struct ParseOutput {
    /// The parsed abstract syntax tree.
    pub ast: FileAst,
    /// Span table mapping SpanIds to byte ranges in the source.
    pub span_table: SpanTable,
    /// All diagnostics collected during lexing and parsing.
    pub symptoms: Vec<Diagnostic>,
}

/// Parse source text and return full results including all diagnostics.
///
/// This is the primary entry point for parsing when you need access to
/// all diagnostic information (errors, warnings, hints).
pub fn parse_source_full(src: &str) -> ParseOutput {
    let mut lexer = Lexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().collect();
    let lex_errors = std::mem::take(&mut lexer.errors);
    let span_table = lexer.take_span_table();

    // Filter out comments for the handwritten parser
    let filtered_tokens: Vec<_> = tokens
        .iter()
        .filter(|(t, _)| !matches!(t, Token::Comment(_)))
        .copied()
        .collect();

    // Clone span_table before passing ownership to parse_tokens_with_errors,
    // since we also need it in the ParseOutput for diagnostic resolution.
    let span_table_clone = span_table.clone();
    let (ast, hw_parse_errors) = expr::parse_tokens_with_errors(&filtered_tokens, span_table);
    let span_table = span_table_clone;

    let mut symptoms: Vec<Diagnostic> = Vec::new();

    // Unterminated strings
    for (_, span_id) in tokens
        .iter()
        .filter(|(t, _)| matches!(t, Token::UnterminatedString(_)))
    {
        symptoms.push(LexSymptom::UnclosedString.into_diagnostic(*span_id));
    }

    // Lex errors
    for (s, span_id) in &lex_errors {
        symptoms.push(s.clone().into_diagnostic(*span_id));
    }

    // Parse errors
    for err in &hw_parse_errors {
        symptoms.push(ParseSymptom::Custom(err.message.clone()).into_diagnostic(err.span_id()));
    }

    // Import validation: quoted local imports must name a `.gin` file.
    for import in ast.uses() {
        for module_import in &import.0 {
            if let ImportSource::Local(path, span_id) = &module_import.source
                && path.extension().is_none_or(|ext| ext != "gin")
            {
                symptoms.push(
                    ParseSymptom::Custom(format!(
                        "local import path must include a `.gin` file name, got `{}`",
                        path.to_string_lossy()
                    ))
                    .into_diagnostic(*span_id),
                );
            }
            if let ImportSource::LocalBundle(b) = &module_import.source
                && module_import.alias.is_some()
            {
                symptoms.push(
                    ParseSymptom::Custom(
                        "`as` alias on `use pkg.(...)` is not supported; use `export as alias` inside the list"
                            .into(),
                    )
                    .into_diagnostic(b.span_id()),
                );
            }
        }
    }

    // Help hints (empty-paren suggestions)
    for (suggested, span_id) in collect_empty_paren_hints(&ast) {
        symptoms.push(ParseSymptom::EmptyParens { suggested }.into_diagnostic(span_id));
    }

    // Unused value info diagnostics
    for (value, span_id) in collect_unused_values(&ast) {
        symptoms.push(ParseSymptom::UnusedValue { value }.into_diagnostic(span_id));
    }

    ParseOutput {
        ast,
        span_table,
        symptoms,
    }
}

/// Extract locally-imported `.gin` file paths from the AST (quoted `use '...gin'` only).
///
/// Package imports and same-folder `use foo` / `use foo.(...)` are not included here.
pub fn extract_local_import_paths(ast: &FileAst, base_dir: &Path) -> Vec<(PathBuf, SpanId)> {
    let mut paths = Vec::new();

    for import in ast.uses() {
        for module_import in &import.0 {
            match &module_import.source {
                ImportSource::Package(_path) => {}
                ImportSource::LocalBundle(_b) => {}
                ImportSource::Local(path, span) => {
                    if path.extension().is_none_or(|e| e != "gin") {
                        continue;
                    }
                    let p = base_dir.join(path);
                    if p.is_file() {
                        paths.push((p, *span));
                    }
                }
            }
        }
    }

    paths
}

/// Resolve package imports (e.g. `use core.io`) against a dependency map.
///
/// `dependencies` maps package names (as declared in `flask.jsonc`) to their
/// on-disk directory paths. For each `ImportSource::Package(path)`:
///
/// - With **no** segments (e.g. `use core`): each file listed in that package's
///   `flask.jsonc` `exports` map (paths relative to the package root).
/// - With **one** segment (e.g. `use core.io`): only `{dep_dir}/{exports["io"].path}` —
///   no scanning for `{dep_dir}/io.gin` or `src/io.gin` unless the export says so.
/// - More than one segment is not supported here (returns nothing).
///
/// Returns `(path, span)` pairs for each resolved `.gin` file.
pub fn extract_package_import_paths(
    ast: &FileAst,
    dependencies: &HashMap<String, PathBuf>,
) -> Vec<(PathBuf, SpanId)> {
    let mut paths = Vec::new();

    for import in ast.uses() {
        for module_import in &import.0 {
            if let ImportSource::Package(mod_path) = &module_import.source {
                let dep_dir = match dependencies.get(mod_path.root.as_str()) {
                    Some(dir) => dir,
                    None => continue,
                };

                if mod_path.segments.is_empty() {
                    if let Some(config) = FlaskConfig::from_directory(dep_dir) {
                        for spec in config.exports().values() {
                            try_push_gin_file(
                                dep_dir.join(&spec.path),
                                mod_path.span_id(),
                                &mut paths,
                            );
                        }
                    }
                } else if mod_path.segments.len() == 1 {
                    let export_key = mod_path.segments[0].as_str();
                    if let Some(config) = FlaskConfig::from_directory(dep_dir)
                        && let Some(spec) = config.exports().get(export_key)
                    {
                        try_push_gin_file(dep_dir.join(&spec.path), mod_path.span_id(), &mut paths);
                    }
                }
                // `use a.b.c` with more than one trailing segment: unsupported for export-only resolution.
            }
        }
    }

    paths
}

/// Push a `.gin` file path onto `paths` if it exists on disk.
fn try_push_gin_file(path: PathBuf, span: SpanId, paths: &mut Vec<(PathBuf, SpanId)>) {
    if path.exists() {
        paths.push((path, span));
    }
}

/// Walk every expression in the AST and collect empty-paren call hints.
fn collect_empty_paren_hints(ast: &FileAst) -> Vec<(String, SpanId)> {
    let mut hints = Vec::new();
    for bind in ast.defs().values() {
        match bind.value() {
            BindValue::Expr(e) => scan_expr(e, &mut hints),
            BindValue::Body { exprs, ret } => {
                for e in exprs {
                    scan_expr(e, &mut hints);
                }
                if let Some(e) = &ret.0 {
                    scan_expr(e, &mut hints);
                }
            }
            BindValue::Extern => {}
        }
    }
    hints
}

fn scan_expr(expr: &Expr, hints: &mut Vec<(String, SpanId)>) {
    match expr {
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                if args.is_empty() {
                    hints.push((fmt_call_without_parens(call), call.path.span_id()));
                }
                for arg in args {
                    scan_expr(arg, hints);
                }
            }
        }
        Expr::Binary(b) => {
            scan_expr(&b.lhs, hints);
            scan_expr(&b.rhs, hints);
        }
        Expr::Bind(b) => match b.value() {
            BindValue::Expr(e) => scan_expr(e, hints),
            BindValue::Body { exprs, ret } => {
                for e in exprs {
                    scan_expr(e, hints);
                }
                if let Some(e) = &ret.0 {
                    scan_expr(e, hints);
                }
            }
            BindValue::Extern => {}
        },
        Expr::When(w) => {
            use ast::WhenArm;
            if let Some(s) = &w.subject {
                scan_expr(s, hints);
            }
            for arm in &w.arms {
                match arm {
                    WhenArm::Cond { condition, body } => {
                        scan_expr(condition, hints);
                        scan_expr(body, hints);
                    }
                    WhenArm::Is { body, .. } | WhenArm::Else(body) => scan_expr(body, hints),
                }
            }
        }
        Expr::TagCall(tc) => {
            for arg in &tc.args {
                scan_expr(arg, hints);
            }
        }
        _ => {}
    }
}

fn fmt_call_without_parens(call: &FnCall) -> String {
    if call.path.segments.is_empty() {
        call.path.root.as_str().to_string()
    } else {
        let segs: Vec<&str> = call.path.segments.iter().map(|s| s.as_str()).collect();
        format!("{}.{}", call.path.root.as_str(), segs.join("."))
    }
}

/// Collect unused top-level expressions as info diagnostics.
///
/// Top-level expressions that don't have their values used may indicate
/// accidental multi-line expressions or missing indentation.
fn collect_unused_values(ast: &FileAst) -> Vec<(String, SpanId)> {
    let mut unused = Vec::new();

    for (expr, expr_span) in ast.top_level_exprs() {
        let (value_str, span) = match expr {
            Expr::Lit(lit) => (format!("{lit:?}"), *expr_span),
            Expr::Binary(_b) => ("binary expression".to_string(), *expr_span),
            Expr::FnCall(call) => {
                let name = if call.path.segments.is_empty() {
                    call.path.root.as_str().to_string()
                } else {
                    let segs: Vec<&str> = call.path.segments.iter().map(|s| s.as_str()).collect();
                    format!("{}.{}", call.path.root.as_str(), segs.join("."))
                };
                (name, call.path.span_id())
            }
            Expr::AnonymousTag(tag, span) => (tag.as_str().to_string(), *span),
            _ => ("expression".to_string(), *expr_span),
        };

        unused.push((value_str, span));
    }

    unused
}
