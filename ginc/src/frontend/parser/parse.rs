//! Parse query - wraps the existing chumsky parser.

use chumsky::error::Rich;
use chumsky::span::SimpleSpan;
use chumsky::{Parser, input::Stream};
use std::path::{Path, PathBuf};

use salsa::Accumulator;

use crate::database::{File, input_database::Db};
use crate::diagnostic::io as io_symptom;
use crate::diagnostic::lex as lex_symptom;
use crate::diagnostic::parse as parse_symptom;
use crate::frontend::lexer::{GinLexer, Token};
use crate::frontend::parser::construct::{FileAst, ImportSource, ModPath as ImportPath};
use crate::frontend::parser::token_parser;

/// Resolve import paths to File inputs for a parsed file.
///
/// This tracked function resolves the imports in the AST to actual File inputs
/// AND accumulates any parse errors into the diagnostics accumulator.
#[salsa::tracked]
pub fn resolve_imports<'db>(db: &'db dyn Db, file: File) -> Vec<File> {
    let ast = parse(db, file);
    extract_import_files(db, &ast, file)
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
}

/// Accumulate all diagnostics from a parse result into the Salsa accumulator.
fn accumulate_diagnostics(db: &dyn Db, parsed: &ParseResult) {
    for span in &parsed.unterminated_strings {
        lex_symptom::unclosed_string(*span).accumulate(db);
    }
    for (msg, span) in &parsed.parse_errors {
        parse_symptom::custom(msg.clone(), *span).accumulate(db);
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
    #[cfg(debug_assertions)]
    let parse_start = std::time::Instant::now();
    #[cfg(debug_assertions)]
    eprintln!("[ginc:parse] start: {:?}", file.path(db));

    let src = file.contents(db);

    let lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer
        .filter(|(t, _)| !matches!(t, Token::Comment(_)))
        .collect();

    // Convert to chumsky stream - extract just the token
    // Chumsky will create synthetic spans based on token index (0, 1, 2, ...)
    // TODO: fix synthetic spans make them real
    let token_stream = Stream::from_iter(tokens.iter().map(|(t, _s)| *t));

    let parser = token_parser();
    let result = parser.parse(token_stream);
    let (maybe_ast, errors) = result.into_output_errors();

    let ast = maybe_ast.unwrap_or_default();

    // early return if no errors
    // spans are only needed for error reporting, so we can avoid the overhead
    if errors.is_empty() {
        return ParseResult {
            ast,
            parse_errors: vec![],
            unterminated_strings: vec![],
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
                ImportSource::Package(path) => {
                    let import_path = resolve_import_path(current_dir, path);
                    match db.input(import_path) {
                        Ok(imported_file) => files.push(imported_file),
                        Err(_) => {
                            // TODO: pass real import span once imports carry span info
                            io_symptom::resolution_failed(SimpleSpan::from(0..0)).accumulate(db);
                        }
                    }
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

/// Convert an import path to a filesystem path.
///
/// TODO: Look up root in flask.json name mappings if not a local folder.
/// For now, treat root as a relative folder name.
fn resolve_import_path(base: &Path, import_path: &ImportPath) -> PathBuf {
    let mut path = base.to_path_buf();

    // Add root
    if !import_path.root.is_empty() {
        path.push(import_path.root.as_str());
    }

    // Add segments
    for segment in &import_path.segments {
        path.push(segment.as_str());
    }

    // Add .gin extension if not present
    if path.extension().is_none() || path.extension().unwrap() != "gin" {
        path.set_extension("gin");
    }

    path
}
