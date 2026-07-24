//! Hover and reference utilities — keyword docs and AST reference collection.
//!
//! These functions support IDE hover popups.

use ast::source::SourceExt;

/// Result of hovering on a keyword or parse-only construct.
pub struct HoverSnippet {
    pub markdown: String,
    pub byte_range: Option<std::ops::Range<usize>>,
}

impl HoverSnippet {
    /// If the word at `byte_pos` is a keyword, return its hover documentation.
    pub fn keyword(source: &str, byte_pos: usize) -> Option<Self> {
        let word = source.word_at_byte_offset(byte_pos)?;
        let word_str = word.as_str();
        for (kw, doc) in KEYWORD_DOCS {
            if *kw == word_str {
                let byte_range = source.word_byte_range(byte_pos).map(|(s, e)| s..e);
                return Some(HoverSnippet {
                    markdown: format!("`{kw}` — {doc}"),
                    byte_range,
                });
            }
        }
        None
    }
}

/// Keyword documentation for parse-level hover.
const KEYWORD_DOCS: &[(&str, &str)] = &[
    ("use", "Import items from other modules into scope"),
    (
        "return",
        "Required in multi-expression functions; carries a value from scope",
    ),
    (
        "if",
        "Guard statement: if condition is met, must return a value",
    ),
    ("else", "Wildcard in `when then else`"),
    ("is", "Pattern matching"),
    (
        "has",
        "Defines properties for an interface (interfaces are compositional)",
    ),
    ("ref", "Immutable borrow"),
    ("eat", "Consume parameter"),
    ("mut", "Mutable reference"),
    ("when", "Pattern matching expression"),
    ("then", "Then branch in pattern matching"),
    ("or", "Variant separator in union types / pattern matching"),
    ("and", "Trait composition"),
    ("for", "Iterate over a range or collection"),
    ("in", "Range membership / iteration"),
    ("while", "Loop with condition"),
    ("loop", "Infinite loop"),
    ("break", "Exit a loop"),
    ("continue", "Skip to next iteration"),
    ("as", "Type cast"),
    ("private", "Private declaration"),
    ("extern", "External function declaration"),
];
