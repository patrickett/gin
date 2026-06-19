//! Cursor-position semantics: completion and find-references (IDE query seam).

use std::ops::Range;

/// Byte ranges of references to a symbol in the current file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencesInFile {
    pub spans: Vec<Range<usize>>,
}
