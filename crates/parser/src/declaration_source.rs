use std::ops::Range;

use lexer::{Lexer, Token};

use crate::cursor::TokenCursor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDeclaration {
    pub owner: Option<String>,
    pub names: Vec<String>,
    pub range: Range<usize>,
}

/// Keeps separate source occurrences before duplicate bindings and type extensions are merged.
/// Validate the source before applying edits: ranges also follow parser recovery on invalid input.
/// Leading ordinary comments and private-section markers remain outside the ranges.
pub fn source_declarations(source: &str) -> Vec<SourceDeclaration> {
    let mut lexer = Lexer::new(source);
    let tokens = std::iter::from_fn(|| lexer.next_raw())
        .filter(|(token, _)| !matches!(token, Token::Comment(_)))
        .collect::<Vec<_>>();
    let mut spans = lexer.take_span_table();
    let mut cursor = TokenCursor::new(&tokens, &mut spans);
    let mut declarations = Vec::new();
    cursor.parse_file_recording(|cursor| cursor.parse_expression(), Some(&mut declarations));
    for declaration in &mut declarations {
        declaration.range = complete_source_line(source, declaration.range.clone());
    }
    declarations
}

fn complete_source_line(source: &str, mut range: Range<usize>) -> Range<usize> {
    let line_start = source[..range.start]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let starts_at_line = source[line_start..range.start].trim().is_empty();
    if starts_at_line {
        range.start = line_start;
    }
    let line_end = source[range.end..]
        .find('\n')
        .map_or(source.len(), |offset| range.end + offset);
    let suffix = source[range.end..line_end].trim_start();
    if suffix.is_empty() || suffix.starts_with("--") {
        range.end = line_end;
        if starts_at_line && line_end < source.len() {
            range.end += 1;
        }
    }
    range
}
