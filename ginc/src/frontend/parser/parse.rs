//! Parse query - wraps the existing chumsky parser.

use chumsky::error::Rich;
use chumsky::span::SimpleSpan;
use chumsky::{Parser, input::Stream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use salsa::Accumulator;

use crate::database::{File, input_database::Db};
use crate::diagnostic::io as io_symptom;
use crate::diagnostic::lex as lex_symptom;
use crate::diagnostic::parse as parse_symptom;
use crate::frontend::lexer::{Token, tokenize};
use crate::frontend::parser::construct::{FileAst, Path as ImportPath};
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
struct ParseResult<'db> {
    ast: FileAst,
    errors: Vec<Rich<'db, Token<'db>>>,
    real_spans: Arc<Vec<SimpleSpan>>,
    unterminated_strings: Vec<SimpleSpan>,
}

/// Accumulate all diagnostics from a parse result into the Salsa accumulator.
fn accumulate_diagnostics(db: &dyn Db, parsed: &ParseResult<'_>) {
    use chumsky::error::RichReason;

    // Unterminated string diagnostics (real byte spans from tokenization)
    for span in &parsed.unterminated_strings {
        lex_symptom::unclosed_string(*span).accumulate(db);
    }

    // Chumsky parse errors (synthetic spans mapped to real byte offsets)
    for err in &parsed.errors {
        let real_span = resolve_span(err.span(), &parsed.real_spans);

        let msg = match err.reason() {
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
        };

        let symptom = parse_symptom::custom(msg, real_span);
        symptom.accumulate(db);
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

/// Internal function to parse the AST (not tracked).
fn parse_ast_internal<'db>(db: &'db dyn Db, file: File) -> ParseResult<'db> {
    let tokens = tokenize(db, file);

    // Collect unterminated string spans — highlight the full token
    let unterminated_strings: Vec<_> = tokens
        .iter()
        .filter(|(t, _)| matches!(t, Token::UnterminatedString(_)))
        .map(|(_, s)| *s)
        .collect();

    // Store the real spans from logos before we lose them
    let real_spans: Vec<_> = tokens.iter().map(|(_, s)| *s).collect();
    let real_spans = Arc::new(real_spans);

    // Convert to chumsky stream - extract just the token
    // Chumsky will create synthetic spans based on token index (0, 1, 2, ...)
    let token_stream = Stream::from_iter(tokens.into_iter().map(|(t, _s)| t));

    // Use existing parser
    let parser = token_parser();
    let result = parser.parse(token_stream);
    let (maybe_ast, errors) = result.into_output_errors();

    let ast = maybe_ast.unwrap_or_default();
    ParseResult {
        ast,
        errors,
        real_spans,
        unterminated_strings,
    }
}

/// Tracked function that computes a hash of the AST for change detection.
#[salsa::tracked]
pub fn ast_hash<'db>(db: &'db dyn Db, file: File) -> u64 {
    let parsed = parse_ast_internal(db, file);
    parsed.ast.compute_content_hash()
}

/// Extract imported File inputs from the imports in an AST.
fn extract_import_files(db: &dyn Db, ast: &FileAst, current_file: File) -> Vec<File> {
    let mut files = Vec::new();
    let current_path = current_file.path(db);
    let current_dir = current_path.parent().unwrap_or(Path::new(""));

    for import in &ast.uses {
        for module_import in &import.0 {
            // Convert import path to filesystem path
            let import_path = resolve_import_path(current_dir, &module_import.path);

            match db.input(import_path) {
                Ok(imported_file) => {
                    files.push(imported_file);
                }
                Err(_) => {
                    let symptom = io_symptom::resolution_failed(SimpleSpan::from(0..0));
                    symptom.accumulate(db);
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
        path.push(&import_path.root);
    }

    // Add segments
    for segment in &import_path.segments {
        path.push(segment);
    }

    // Add .gin extension if not present
    if path.extension().is_none() || path.extension().unwrap() != "gin" {
        path.set_extension("gin");
    }

    path
}
