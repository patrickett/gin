//! Pure parsing functions — no Salsa or database dependency.
//!
//! This module exposes pure functions that parse source text and extract
//! information from ASTs without relying on incremental compilation
//! infrastructure.

use ast::span::{SpanId, SpanTable};
use diagnostic::lex::LexSymptom;
use lexer::{Lexer, Token};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::cursor::ParseError;
use crate::expr;
use ast::{BindValue, Expr, FileAst, FnCall, ImportSource};

// TODO: change ParseOutput to a Vec<Symptom> or some vec of enum so we dont need the different
// fields
/// Full output from parsing source text, including all diagnostic info.
pub struct ParseOutput {
    /// The parsed abstract syntax tree.
    pub ast: FileAst,
    /// Span table mapping SpanIds to byte ranges in the source.
    pub span_table: SpanTable,
    /// Errors encountered during parsing.
    pub parse_errors: Vec<ParseError>,
    /// Errors encountered during lexing, paired with their span.
    pub lex_errors: Vec<(LexSymptom, SpanId)>,
    /// SpanIds for unterminated string literals.
    pub unterminated_strings: Vec<SpanId>,
    /// Empty-paren hints: (suggested form, span ID).
    pub help_hints: Vec<(String, SpanId)>,
    /// Unused value info diagnostics: (value description, span ID).
    pub unused_values: Vec<(String, SpanId)>,
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

    let help_hints = collect_empty_paren_hints(&ast);
    let unused_values = collect_unused_values(&ast);

    let unterminated_strings: Vec<_> = tokens
        .iter()
        .filter(|(t, _)| matches!(t, Token::UnterminatedString(_)))
        .map(|(_, s)| *s)
        .collect();

    ParseOutput {
        ast,
        span_table,
        parse_errors: hw_parse_errors,
        unterminated_strings,
        lex_errors,
        help_hints,
        unused_values,
    }
}

/// Extract local import directory paths from the AST, relative to the given directory.
///
/// Returns `(path, span)` pairs for each `.gin` file found in locally-imported
/// directories. Package imports are skipped — use [`extract_package_import_paths`]
/// to resolve those separately with access to the dependency map.
pub fn extract_local_import_paths(ast: &FileAst, base_dir: &Path) -> Vec<(PathBuf, SpanId)> {
    let mut paths = Vec::new();

    for import in ast.uses() {
        for module_import in &import.0 {
            match &module_import.source {
                ImportSource::Package(_path) => {
                    // Package imports from flask.json dependencies are not resolved
                    // by this pure function. Callers that need package resolution
                    // should handle it separately with access to the dependency map.
                }
                ImportSource::Local(path, span) => {
                    let folder = base_dir.join(path);
                    // TODO: PERF can we mmap files directly instead of individual read_dirs
                    match std::fs::read_dir(&folder) {
                        Ok(entries) => {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.extension().is_some_and(|e| e == "gin") {
                                    paths.push((p, *span));
                                }
                            }
                        }
                        Err(_) => {
                            // Caller is responsible for handling resolution failures
                            // since we don't have access to a diagnostic emitter here.
                        }
                    }
                }
            }
        }
    }

    paths
}

/// Resolve package imports (e.g. `use core.io`) against a dependency map.
///
/// `dependencies` maps package names (as declared in `flask.json`) to their
/// on-disk directory paths. For each `ImportSource::Package(path)`:
///
/// - With segments (e.g. `core.io`): looks for `{dep_dir}/io.gin` and
///   `{dep_dir}/src/io.gin`, including both if they exist.
/// - Without segments (e.g. `core`): includes every `.gin` file found in
///   `{dep_dir}/` and `{dep_dir}/src/`.
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
                    // `use core` — include all .gin files in the package
                    collect_gin_files_from_dir(dep_dir, mod_path.span, &mut paths);
                    let src_dir = dep_dir.join("src");
                    if src_dir.is_dir() {
                        collect_gin_files_from_dir(&src_dir, mod_path.span, &mut paths);
                    }
                } else {
                    // `use core.io` — resolve each segment to a file
                    for segment in &mod_path.segments {
                        try_push_gin_file(
                            dep_dir.join(format!("{}.gin", segment)),
                            mod_path.span,
                            &mut paths,
                        );
                        try_push_gin_file(
                            dep_dir.join("src").join(format!("{}.gin", segment)),
                            mod_path.span,
                            &mut paths,
                        );
                    }
                }
            }
        }
    }

    paths
}

/// Collect all `.gin` files from a directory into `paths`.
fn collect_gin_files_from_dir(dir: &Path, span: SpanId, paths: &mut Vec<(PathBuf, SpanId)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "gin") {
            paths.push((p, span));
        }
    }
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
                    hints.push((fmt_call_without_parens(call), call.path.span));
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
                (name, call.path.span)
            }
            Expr::AnonymousTag(tag, span) => (tag.as_str().to_string(), *span),
            _ => ("expression".to_string(), *expr_span),
        };

        unused.push((value_str, span));
    }

    unused
}
