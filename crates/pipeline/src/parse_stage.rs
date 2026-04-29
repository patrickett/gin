use crate::{ParseResult, ParsedFile, SourceCollection};
use parser::parse_source_full;

/// Read and parse all source files in the collection.
///
/// Each file is read from disk and parsed into an AST. Parse diagnostics
/// are accumulated in the result. Callers should check `has_fatal()` before
/// proceeding to import resolution.
pub fn parse(collection: SourceCollection) -> ParseResult {
    let mut files = Vec::with_capacity(collection.file_paths.len());
    let mut diagnostics = Vec::new();

    for fp in &collection.file_paths {
        let source = match std::fs::read_to_string(fp) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("Error reading {}: {}", fp.display(), err);
                continue;
            }
        };
        let output = parse_source_full(&source);
        diagnostics.extend(output.symptoms.clone());
        files.push(ParsedFile {
            path: fp.clone(),
            source,
            output,
        });
    }

    ParseResult {
        files,
        diagnostics,
    }
}
