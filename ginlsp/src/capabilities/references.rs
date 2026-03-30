use crate::diagnostics::span_to_range;
use ginc::FileAst;
use tower_lsp::lsp_types::{Location, Url};

pub fn find_all_references(source: &str, ast: &FileAst, word: &str, uri: &Url) -> Vec<Location> {
    ginc::find_references(ast, word)
        .into_iter()
        .map(|span| Location {
            uri: uri.clone(),
            range: span_to_range(span.start, span.end, source),
        })
        .collect()
}
