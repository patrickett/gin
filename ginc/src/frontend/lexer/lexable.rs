use crate::{frontend::GinLexer, source::Source};
use std::path::PathBuf;

// PERF: Consider using iterator adapters that avoid intermediate Vec allocation when possible
pub trait Lexable<S> {
    fn lex(&self) -> Vec<(GinLexer<'_>, PathBuf)>;
}

impl<S: Source> Lexable<S> for S {
    fn lex(&self) -> Vec<(GinLexer<'_>, PathBuf)> {
        let source_codes = self.content();

        source_codes
            .into_iter()
            .map(|(content, path)| (GinLexer::new_owned(content), path))
            .collect::<Vec<(GinLexer, PathBuf)>>()
    }
}
