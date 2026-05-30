//! Pure parsing functions — no Salsa or database dependency.
//!
//! This module exposes pure functions that parse source text and extract
//! information from ASTs without relying on incremental compilation
//! infrastructure.

use ast::folder::{Folder, walk_file_ast_mut, walk_fn_call_mut};
use ast::span::{HasSpanId, SpanId};
use diagnostic::LexSymptom;
use diagnostic::parse::ParseSymptom;
use diagnostic::{Diagnostic, DiagnosticLike, UseSymptom};
use lexer::{Lexer, Token};
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use crate::expr;
use ast::{Expr, FileAst, FnCall, ImportSource};
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

    let mut symptoms: Vec<Diagnostic> = Vec::new();

    // Unterminated strings
    for (_, span_id) in tokens
        .iter()
        .filter(|(t, _)| matches!(t, Token::UnterminatedString(_)))
    {
        symptoms.push(LexSymptom::UnclosedString.into_diagnostic(span_table.get(*span_id)));
    }

    // Lex errors
    for (s, span_id) in &lex_errors {
        symptoms.push(s.clone().into_diagnostic(span_table.get(*span_id)));
    }

    // Parse errors — some carry magic prefixes that indicate non-fatal hints.
    for err in &hw_parse_errors {
        if let Some("indented-return") = err.message.strip_prefix("__gin_hint__:") {
            symptoms.push(ParseSymptom::IndentedReturn.into_diagnostic(span_table.get(err.span)));
        } else {
            symptoms.push(
                ParseSymptom::Custom(err.message.clone()).into_diagnostic(span_table.get(err.span)),
            );
        }
    }

    ast.span_table = span_table;

    // Help hints (empty-paren suggestions) — compute before spans borrows ast.
    let hints = collect_empty_paren_hints(&mut ast);

    let spans = ast.span_table();

    // Import validation
    for import in ast.uses() {
        for module_import in &import.0 {
            match &module_import.source {
                ImportSource::LocalBundle(b) if module_import.alias.is_some() => {
                    symptoms.push(
                        ParseSymptom::Custom(
                            "`as` alias on `use pkg.(...)` is not supported; use `export as alias` inside the list"
                                .into(),
                        )
                        .into_diagnostic(spans.get(b.span_id())),
                    );
                }
                ImportSource::Local(path, span) if module_import.alias.is_none() => {
                    symptoms.push(
                        ParseSymptom::Custom(format!(
                            "folder module import requires `as` (e.g. `use '{}' as name`)",
                            path.display()
                        ))
                        .into_diagnostic(spans.get(*span)),
                    );
                }
                ImportSource::LocalBundle(b) => {
                    collect_prefer_member_import_hints(b, spans, &mut symptoms);
                }
                _ => {}
            }
        }
    }

    for (suggested, span_id) in &hints {
        symptoms.push(
            ParseSymptom::EmptyParens {
                suggested: suggested.clone(),
            }
            .into_diagnostic(spans.get(*span_id)),
        );
    }

    // Unused value info diagnostics
    for (value, span_id) in collect_unused_values(&ast) {
        symptoms.push(ParseSymptom::UnusedValue { value }.into_diagnostic(spans.get(span_id)));
    }

    ParseOutput { ast, symptoms }
}

fn collect_prefer_member_import_hints(
    b: &ast::LocalBundleImport,
    span_table: &ast::span::SpanTable,
    symptoms: &mut Vec<Diagnostic>,
) {
    if b.members.len() != 1 {
        return;
    }
    let member = &b.members[0];
    if member.alias.is_some() {
        return;
    }
    let path_prefix = if let Some(p) = &b.local_path {
        format!("'{}'", p.to_string_lossy())
    } else {
        b.qualifier_prefix()
    };
    symptoms.push(
        UseSymptom::PreferMemberImport {
            path_prefix,
            symbol: member.export.to_string(),
        }
        .into_diagnostic(span_table.get(member.span)),
    );
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
                ImportSource::LocalMember(_) => {}
                ImportSource::CurrentModule { .. } => {}
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
                    let module_dir = b
                        .path_segments
                        .iter()
                        .fold(dep_dir.clone(), |dir, seg| dir.join(seg.as_str()));
                    if !module_dir.is_dir() {
                        continue;
                    }
                    let span = b.span_id();
                    for m in &b.members {
                        let nested = module_dir.join(m.export.as_str());
                        if nested.join(PACKAGE_CONFIG_NAME).is_file() {
                            for p in list_package_gin_files(&nested) {
                                paths.push((p, span));
                            }
                        }
                    }
                }
                ImportSource::Local(_, _) => {}
                ImportSource::LocalMember(_) => {}
                ImportSource::CurrentModule { .. } => {}
            }
        }
    }

    paths
}

/// Walk every expression in the AST and collect empty-paren call hints.
fn collect_empty_paren_hints(ast: &mut FileAst) -> Vec<(String, SpanId)> {
    let mut collector = EmptyParenCollector { hints: Vec::new() };
    let _ = walk_file_ast_mut(&mut collector, ast);
    collector.hints
}

struct EmptyParenCollector {
    hints: Vec<(String, SpanId)>,
}

impl Folder for EmptyParenCollector {
    fn visit_fn_call(&mut self, call: &mut ast::FnCall) -> ControlFlow<()> {
        if let Some(args) = &call.args
            && args.is_empty()
        {
            self.hints
                .push((fmt_call_without_parens(call), call.path.span_id()));
        }
        walk_fn_call_mut(self, call)
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
            Expr::AnonymousTag(tag) => (tag.as_str().to_string(), *expr_span),
            _ => ("expression".to_string(), *expr_span),
        };

        unused.push((value_str, span));
    }

    unused
}
