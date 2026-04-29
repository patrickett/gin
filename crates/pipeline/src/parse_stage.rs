use crate::ParsedFile;
use parser::parse_source_full;
use std::path::PathBuf;

/// Parse source texts into ASTs.
///
/// Each `(PathBuf, String)` pair is a file path and its contents.
/// Parse diagnostics are stored in each `ParsedFile`'s `output.symptoms`.
pub fn parse(sources: &[(PathBuf, String)]) -> Vec<ParsedFile> {
    sources
        .iter()
        .map(|(path, source)| {
            let output = parse_source_full(source);
            ParsedFile {
                path: path.clone(),
                source: source.clone(),
                output,
            }
        })
        .collect()
}
