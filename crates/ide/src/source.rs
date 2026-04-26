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

pub fn get_word_at_position(source: &str, line: u32, character: u32) -> Option<String> {
    let byte_idx = position_to_byte_offset(source, line, character)?;
    word_at_byte_offset(source, byte_idx)
}

/// Extract a full numeric literal at `(line, character)`, including an optional leading `-`
/// and a decimal point (e.g. `-3`, `3.14`, `-0.5`).
/// Returns `None` if the cursor is not on a digit or a `-` immediately before digits.
pub fn get_number_at_position(source: &str, line: u32, character: u32) -> Option<String> {
    let byte_idx = position_to_byte_offset(source, line, character)?;

    let bytes = source.as_bytes();

    let cursor = *bytes.get(byte_idx)? as char;
    let is_on_digit = cursor.is_ascii_digit();
    let is_on_minus = cursor == '-'
        && bytes
            .get(byte_idx + 1)
            .is_some_and(|&b| (b as char).is_ascii_digit());
    let is_on_dot = cursor == '.'
        && byte_idx > 0
        && (bytes[byte_idx - 1] as char).is_ascii_digit()
        && bytes
            .get(byte_idx + 1)
            .is_some_and(|&b| (b as char).is_ascii_digit());

    if !is_on_digit && !is_on_minus && !is_on_dot {
        return None;
    }

    let mut start = byte_idx;
    while start > 0 && (bytes[start - 1] as char).is_ascii_digit() {
        start -= 1;
    }
    if start > 0 && bytes[start - 1] as char == '-' {
        start -= 1;
    }

    let mut end = byte_idx;
    while end < bytes.len() && (bytes[end] as char).is_ascii_digit() {
        end += 1;
    }
    if end < bytes.len() && bytes[end] as char == '.' {
        let after_dot = end + 1;
        if after_dot < bytes.len() && (bytes[after_dot] as char).is_ascii_digit() {
            end += 1;
            while end < bytes.len() && (bytes[end] as char).is_ascii_digit() {
                end += 1;
            }
        }
    }

    if start == end {
        return None;
    }

    // If the numeric token is part of an identifier (e.g. `Tag123`), do not treat it
    // as a numeric literal. This prevents number-hover from stealing hover for tags.
    if start > 0 && is_identifier_char(bytes[start - 1] as char) {
        return None;
    }
    if end < bytes.len() && is_identifier_char(bytes[end] as char) {
        return None;
    }

    Some(source[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::get_number_at_position;

    #[test]
    fn number_at_position_rejects_digits_inside_identifiers() {
        let src = "Tag123 other 456";
        // Cursor on the '1' in Tag123
        assert_eq!(get_number_at_position(src, 0, 3), None);
        // Cursor on the '3' in Tag123
        assert_eq!(get_number_at_position(src, 0, 5), None);
        // Cursor on the '4' in 456
        assert_eq!(get_number_at_position(src, 0, 14), Some("456".to_string()));
    }
}

pub fn get_char_at_position(source: &str, line: u32, character: u32) -> Option<char> {
    let byte_idx = position_to_byte_offset(source, line, character)?;
    source.as_bytes().get(byte_idx).map(|&b| b as char)
}

/// Result of detecting a string literal at a cursor position.
pub struct StringLiteralInfo {
    /// Byte range of the entire string token including quotes.
    pub range: std::ops::Range<usize>,
    /// The string content without quotes.
    pub content: String,
}

/// If `byte_pos` is inside a single-quoted string literal (non-template),
/// return information about it. Returns `None` if the position is inside a
/// format string or not inside any string at all.
pub fn get_string_literal_at(source: &str, byte_pos: usize) -> Option<StringLiteralInfo> {
    let bytes = source.as_bytes();

    // Scan backwards for an odd number of consecutive single quotes.
    // An odd count means we're inside a string opened by that quote.
    let mut pos = byte_pos;
    let mut quote_pos: Option<usize> = None;

    while pos > 0 {
        pos -= 1;
        if bytes[pos] == b'\'' {
            // Count consecutive quotes at this position
            let mut count = 1;
            let mut p = pos;
            while p > 0 && bytes[p - 1] == b'\'' {
                p -= 1;
                count += 1;
            }
            if count % 2 == 1 {
                // Odd number — this opens a string
                quote_pos = Some(pos);
                break;
            }
            // Even number — these are escaped quotes, keep scanning
            pos = p;
        } else if bytes[pos] == b'\n' {
            // Plain strings can't span lines
            break;
        }
    }

    let open = quote_pos?;

    // Find the closing quote
    let mut close = open + 1;
    while close < bytes.len() && bytes[close] != b'\'' && bytes[close] != b'\n' {
        close += 1;
    }

    if close >= bytes.len() || bytes[close] != b'\'' {
        // Unterminated string
        return None;
    }

    // Check byte_pos is within the token span (including quotes)
    if byte_pos < open || byte_pos > close + 1 {
        return None;
    }

    // Make sure this isn't inside a format string (double-quoted)
    // Check if there's an unmatched " before the open quote on the same line
    let line_start = source[..=open]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let text_before = &source[line_start..open];
    let double_quotes = text_before.chars().filter(|&c| c == '"').count();
    if double_quotes % 2 == 1 {
        // Inside a format string
        return None;
    }

    let content = source[open + 1..close].to_string();

    Some(StringLiteralInfo {
        range: open..close + 1,
        content,
    })
}
