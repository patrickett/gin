use crate::diagnostics::span_to_range;
use ginc::FileAst;
use tower_lsp::lsp_types::Range;

pub fn find_definition_range(source: &str, ast: &FileAst, word: &str) -> Range {
    ginc::find_definition_span(ast, word)
        .map(|span| span_to_range(span.start, span.end, source))
        .unwrap_or_default()
}
