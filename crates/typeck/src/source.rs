//! Source code utilities for LSP (position conversion, word extraction, etc.)

use lexer::is_comment_at;

/// Convert a byte offset to (line, column) position.
///
/// Column is measured in UTF-16 code units (LSP specification requirement).
pub fn byte_offset_to_position(byte: usize, source: &str) -> (u32, u32) {
    let mut line = 0u32;
    let mut col = 0u32;
    let mut current_byte = 0usize;

    for ch in source.chars() {
        if current_byte >= byte {
            break;
        }

        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += ch.len_utf16() as u32;
        }
        current_byte += ch.len_utf8();
    }

    (line, col)
}

pub fn position_to_byte_offset(source: &str, line: u32, character: u32) -> Option<usize> {
    let line_start: usize = source
        .split('\n')
        .take(line as usize)
        .map(|l| l.len() + 1)
        .sum();
    if line_start > source.len() {
        return None;
    }
    let mut utf16_units = 0u32;
    for (byte_idx, c) in source[line_start..].char_indices() {
        if utf16_units == character {
            return Some(line_start + byte_idx);
        }
        utf16_units += c.len_utf16() as u32;
    }
    (utf16_units == character).then_some(line_start + source[line_start..].len())
}

pub fn is_in_comment(source: &str, line: u32, character: u32) -> bool {
    let Some(byte_pos) = position_to_byte_offset(source, line, character) else {
        return false;
    };
    is_comment_at(source, byte_pos)
}

pub fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn word_at_byte_offset(source: &str, byte_pos: usize) -> Option<String> {
    let mut start = byte_pos;
    let mut end = byte_pos;
    let bytes = source.as_bytes();
    while start > 0 && is_identifier_char(bytes[start - 1] as char) {
        start -= 1;
    }
    while end < bytes.len() && is_identifier_char(bytes[end] as char) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(source[start..end].to_string())
}

pub fn get_char_at_position(source: &str, line: u32, character: u32) -> Option<char> {
    let byte_idx = position_to_byte_offset(source, line, character)?;
    source.as_bytes().get(byte_idx).map(|&b| b as char)
}
