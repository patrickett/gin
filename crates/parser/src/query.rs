//! Pure parsing functions — no Salsa or database dependency.
//!
//! This module exposes pure functions that parse source text and extract
//! information from ASTs without relying on incremental compilation
//! infrastructure.

use ast::span::{HasSpanId, SpanId};
use diagnostic::LexSymptom;
use diagnostic::parse::ParseSymptom;
use diagnostic::{Diagnostic, DiagnosticLike};
use lexer::{Lexer, Token};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::expr;
use ast::{BindValue, Expr, FileAst, FnCall, ImportSource};
use flask::{
    NestedPackageTarget, PACKAGE_CONFIG_NAME, list_package_gin_files, resolve_nested_package_path,
};

// TODO: change ParseOutput to a Vec<Diagnostic> or some vec of enum so we dont need the different
// fields
/// Full output from parsing source text, including all diagnostic info.
#[derive(Clone, PartialEq, Eq)]
pub struct ParseOutput {
    /// The parsed abstract syntax tree.
    pub ast: FileAst,
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
    let mut span_table = lexer.take_span_table();

    // Filter out comments for the handwritten parser
    let filtered_tokens: Vec<_> = tokens
        .iter()
        .filter(|(t, _)| !matches!(t, Token::Comment(_)))
        .copied()
        .collect();

    let (mut ast, hw_parse_errors) =
        expr::parse_tokens_with_errors(&filtered_tokens, &mut span_table);
    ast.span_table = span_table;

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

    // Import validation: `use pkg.(...)` does not support a top-level `as`.
    for import in ast.uses() {
        for module_import in &import.0 {
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

    ParseOutput { ast, symptoms }
}

/// Extract locally-imported `.gin` file paths from the AST (quoted `use '...gin'` only).
///
/// Package imports, dependency bundles (`use dep.(...)`), and unquoted local roots are not included here.
pub fn extract_local_import_paths(ast: &FileAst, base_dir: &Path) -> Vec<(PathBuf, SpanId)> {
    let mut paths = Vec::new();

    for import in ast.uses() {
        for module_import in &import.0 {
            match &module_import.source {
                ImportSource::Package(_path) => {}
                ImportSource::LocalBundle(_b) => {}
                ImportSource::Local(path, span) => {
                    let p = base_dir.join(path);
                    if p.is_file() && p.extension().is_some_and(|e| e == "gin") {
                        paths.push((p, *span));
                    } else if p.is_dir() && p.join(PACKAGE_CONFIG_NAME).is_file() {
                        for g in list_package_gin_files(&p) {
                            paths.push((g, *span));
                        }
                    }
                }
            }
        }
    }

    paths
}

/// Resolve package imports (e.g. `use core`, `use core.subpkg`) against a dependency map.
///
/// - `use dep`: every `*.gin` in `dep/` next to `flask.jsonc` (non-recursive).
/// - `use dep.a.b`: nested folder modules `dep/a/b/` with `flask.jsonc`, then flat `*.gin` there.
/// - `use dep.(a, b)`: each listed name is a nested folder module under `dep/`.
///
/// Returns `(path, span)` pairs for each resolved `.gin` file.
pub fn extract_package_import_paths(
    ast: &FileAst,
    dependencies: &HashMap<String, PathBuf>,
) -> Vec<(PathBuf, SpanId)> {
    let mut paths = Vec::new();

    for import in ast.uses() {
        for module_import in &import.0 {
            match &module_import.source {
                ImportSource::Package(mod_path) => {
                    let dep_dir = match dependencies.get(mod_path.root.as_str()) {
                        Some(dir) => dir,
                        None => continue,
                    };

                    let span = mod_path.span_id();
                    if mod_path.segments.is_empty() {
                        if dep_dir.join(PACKAGE_CONFIG_NAME).is_file() {
                            for p in list_package_gin_files(dep_dir) {
                                paths.push((p, span));
                            }
                        }
                        continue;
                    }

                    let segs: Vec<&str> = mod_path.segments.iter().map(|s| s.as_str()).collect();
                    if let Ok(NestedPackageTarget::FolderModule(dir)) =
                        resolve_nested_package_path(dep_dir, &segs)
                    {
                        for p in list_package_gin_files(&dir) {
                            paths.push((p, span));
                        }
                    }
                }
                ImportSource::LocalBundle(b) => {
                    let dep_dir = match dependencies.get(b.root.as_str()) {
                        Some(dir) => dir,
                        None => continue,
                    };
                    if !dep_dir.join(PACKAGE_CONFIG_NAME).is_file() {
                        continue;
                    }
                    let span = b.span_id();
                    for m in &b.members {
                        let nested = dep_dir.join(m.export.as_str());
                        if nested.join(PACKAGE_CONFIG_NAME).is_file() {
                            for p in list_package_gin_files(&nested) {
                                paths.push((p, span));
                            }
                        }
                    }
                }
                ImportSource::Local(_, _) => {}
            }
        }
    }

    paths
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
