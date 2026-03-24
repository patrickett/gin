//! Parse query - wraps the existing chumsky parser.

use crate::ast::{BindValue, Expr, FileAst, FnCall, ImportSource, token_parser};
use crate::database::{File, input_database::Db};
use crate::diagnostic::io as io_symptom;
use crate::diagnostic::lex as lex_symptom;
use crate::diagnostic::lex::LexSymptom;
use crate::diagnostic::parse as parse_symptom;
use crate::diagnostic::{Category, Symptom, SymptomSource};
use crate::lexer::{GinLexer, Token};
use chumsky::error::Rich;
use chumsky::span::SimpleSpan;
use chumsky::{Parser, input::Stream};
use salsa::Accumulator;
use std::path::Path;

/// Resolve import paths to File inputs for a parsed file.
///
/// This tracked function resolves the imports in the AST to actual File inputs
/// AND accumulates any parse errors into the diagnostics accumulator.
#[salsa::tracked]
pub fn resolve_imports<'db>(db: &'db dyn Db, file: File) -> Vec<File> {
    let ast = parse(db, file);
    extract_import_files(db, &ast, file)
}

/// Parse a Gin source string without the Salsa incremental system.
///
/// Used for dependency loading at build time where caching is unnecessary.
pub fn parse_from_str(src: &str) -> FileAst {
    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer
        .by_ref()
        .filter(|(t, _)| !matches!(t, Token::Comment(_)))
        .collect();
    let token_stream = Stream::from_iter(tokens.iter().map(|(t, _s)| *t));
    let parser = token_parser();
    let (maybe_ast, _) = parser.parse(token_stream).into_output_errors();
    maybe_ast.unwrap_or_default()
}

/// Parse a Gin source file and return the AST as a tracked query.
///
/// The AST is cached and only recomputed when the input changes.
#[salsa::tracked]
pub fn parse<'db>(db: &'db dyn Db, file: File) -> FileAst {
    let parsed = parse_ast_internal(db, file);
    accumulate_diagnostics(db, &parsed);
    parsed.ast
}

/// Result of parsing a file (not tracked).
struct ParseResult {
    ast: FileAst,
    /// Pre-formatted error messages with their real byte spans.
    parse_errors: Vec<(String, SimpleSpan)>,
    unterminated_strings: Vec<SimpleSpan>,
    lex_errors: Vec<(LexSymptom, SimpleSpan)>,
    /// Empty-paren hints: (suggested form, span).
    help_hints: Vec<(String, SimpleSpan)>,
    /// Unused value info diagnostics: (value description, span).
    unused_values: Vec<(String, SimpleSpan)>,
}

/// Accumulate all diagnostics from a parse result into the Salsa accumulator.
fn accumulate_diagnostics(db: &dyn Db, parsed: &ParseResult) {
    for span in &parsed.unterminated_strings {
        lex_symptom::unclosed_string(*span).accumulate(db);
    }
    for (symptom, span) in &parsed.lex_errors {
        Symptom {
            source: SymptomSource::Lex(symptom.clone()),
            span: *span,
            category: Category::Flaw,
        }
        .accumulate(db);
    }
    for (msg, span) in &parsed.parse_errors {
        parse_symptom::custom(msg.clone(), *span).accumulate(db);
    }
    for (suggested, span) in &parsed.help_hints {
        parse_symptom::empty_parens_hint(suggested.clone(), *span).accumulate(db);
    }
    for (value, span) in &parsed.unused_values {
        parse_symptom::unused_value(value.clone(), *span).accumulate(db);
    }
}

/// Map a chumsky synthetic span (token indices) to real byte offsets.
///
/// Chumsky's `Stream::from_iter` assigns each token index 0, 1, 2, etc.
/// The `real_spans` table maps token index → the actual byte span from logos.
fn resolve_span(synth: &SimpleSpan, real_spans: &[SimpleSpan]) -> SimpleSpan {
    let start_idx = synth.start;
    let end_idx = synth.end.saturating_sub(1); // chumsky end is exclusive

    let start = real_spans.get(start_idx).map_or(0, |s| s.start);
    let end = real_spans.get(end_idx).map_or(start, |s| s.end);

    SimpleSpan::from(start..end)
}

/// Format a chumsky Rich error into an owned string.
fn format_rich_error(err: &Rich<'_, Token<'_>>) -> String {
    use chumsky::error::RichReason;
    match err.reason() {
        RichReason::ExpectedFound { expected, found } => {
            let expected_str = if expected.is_empty() {
                "something else".to_string()
            } else {
                expected
                    .iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            match found {
                Some(tok) => format!("expected {expected_str}, found {tok:?}"),
                None => format!("expected {expected_str}, found end of input"),
            }
        }
        RichReason::Custom(msg) => msg.clone(),
    }
}

/// Internal function to parse the AST (not tracked).
fn parse_ast_internal(db: &dyn Db, file: File) -> ParseResult {
    let src = file.contents(db);

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer
        .by_ref()
        .filter(|(t, _)| !matches!(t, Token::Comment(_)))
        .collect();
    let lex_errors = lexer.errors;

    // Convert to chumsky stream - extract just the token
    // Chumsky will create synthetic spans based on token index (0, 1, 2, ...)
    // TODO: fix synthetic spans make them real
    let token_stream = Stream::from_iter(tokens.iter().map(|(t, _s)| *t));

    let parser = token_parser();
    let result = parser.parse(token_stream);
    let (maybe_ast, errors) = result.into_output_errors();

    let ast = maybe_ast.unwrap_or_default();

    let help_hints = collect_empty_paren_hints(&ast);
    let unused_values = collect_unused_values(&ast);

    // early return if no errors
    // spans are only needed for error reporting, so we can avoid the overhead
    if errors.is_empty() {
        return ParseResult {
            ast,
            parse_errors: vec![],
            unterminated_strings: vec![],
            lex_errors,
            help_hints,
            unused_values,
        };
    }

    let unterminated_strings: Vec<_> = tokens
        .iter()
        .filter(|(t, _)| matches!(t, Token::UnterminatedString(_)))
        .map(|(_, s)| *s)
        .collect();

    let real_spans: Vec<_> = tokens.iter().map(|(_, s)| *s).collect();

    let parse_errors: Vec<(String, SimpleSpan)> = errors
        .iter()
        .map(|err| {
            let real_span = resolve_span(err.span(), &real_spans);
            let msg = format_rich_error(err);
            (msg, real_span)
        })
        .collect();

    ParseResult {
        ast,
        parse_errors,
        unterminated_strings,
        lex_errors,
        help_hints,
        unused_values,
    }
}

/// Tracked function that computes a hash of the AST for change detection.
#[salsa::tracked]
pub fn ast_hash<'db>(db: &'db dyn Db, file: File) -> u64 {
    let ast = parse(db, file);
    ast.compute_content_hash()
}

/// Extract imported File inputs from the imports in an AST.
fn extract_import_files(db: &dyn Db, ast: &FileAst, current_file: File) -> Vec<File> {
    let mut files = Vec::new();
    let current_path = current_file.path(db);
    let current_dir = current_path.parent().unwrap_or(Path::new(""));

    for import in ast.uses() {
        for module_import in &import.0 {
            match &module_import.source {
                ImportSource::Package(_path) => {
                    // Package imports are resolved by flask.json at native build time
                    // via load_entry_with_deps — not by the salsa pipeline.
                }
                ImportSource::Local(path) => {
                    let folder = current_dir.join(path);
                    // TODO: PERF can we mmap files directly instead of individual read_dirs
                    match std::fs::read_dir(&folder) {
                        Ok(entries) => {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.extension().is_some_and(|e| e == "gin") {
                                    match db.input(p) {
                                        Ok(f) => files.push(f),
                                        Err(_) => {
                                            // TODO: pass real import span once imports carry span info
                                            io_symptom::resolution_failed(SimpleSpan::from(0..0))
                                                .accumulate(db);
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // TODO: pass real import span once imports carry span info
                            io_symptom::resolution_failed(SimpleSpan::from(0..0)).accumulate(db);
                        }
                    }
                }
            }
        }
    }

    files
}

/// Walk every expression in the AST and collect empty-paren call hints.
fn collect_empty_paren_hints(ast: &FileAst) -> Vec<(String, SimpleSpan)> {
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

fn scan_expr(expr: &Expr, hints: &mut Vec<(String, SimpleSpan)>) {
    match expr {
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                if args.is_empty() {
                    hints.push((fmt_call_without_parens(call), SimpleSpan::from(0..0)));
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
            use crate::ast::when::WhenArm;
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
fn collect_unused_values(ast: &FileAst) -> Vec<(String, SimpleSpan)> {
    let mut unused = Vec::new();

    for expr in ast.top_level_exprs() {
        // Format the expression for display
        let value_str = match expr {
            Expr::Lit(lit) => format!("{lit:?}"),
            Expr::Binary(_b) => "binary expression".to_string(),
            Expr::FnCall(call) => {
                if call.path.segments.is_empty() {
                    call.path.root.as_str().to_string()
                } else {
                    let segs: Vec<&str> = call.path.segments.iter().map(|s| s.as_str()).collect();
                    format!("{}.{}", call.path.root.as_str(), segs.join("."))
                }
            }
            Expr::AnonymousTag(tag) => tag.as_str().to_string(),
            _ => "expression".to_string(),
        };

        // Use 0..0 as span since we don't have real span info for top-level exprs
        // TODO: track real spans for top-level expressions
        unused.push((value_str, SimpleSpan::from(0..0)));
    }

    unused
}
