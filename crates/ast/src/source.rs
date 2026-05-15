//! Source code utilities for LSP (position conversion, word extraction, etc.)

/// Check whether `byte_pos` falls on a line that already has `--` before it
/// (i.e. is inside a line comment).
fn is_comment_at(source: &str, byte_pos: usize) -> bool {
    let line_start = source[..byte_pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &source[line_start..];
    if let Some(comment_start) = line.find("--") {
        let comment_byte = line_start + comment_start;
        byte_pos >= comment_byte
    } else {
        false
    }
}

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
    let (start, end) = word_byte_range(source, byte_pos)?;
    Some(source[start..end].to_string())
}

/// Return the (start, end) byte range of the identifier word at `byte_pos`.
/// Returns `None` when the cursor is not on an identifier character.
pub fn word_byte_range(source: &str, byte_pos: usize) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    if byte_pos >= bytes.len() || !is_identifier_char(bytes[byte_pos] as char) {
        return None;
    }
    let mut start = byte_pos;
    let mut end = byte_pos;
    while start > 0 && is_identifier_char(bytes[start - 1] as char) {
        start -= 1;
    }
    while end < bytes.len() && is_identifier_char(bytes[end] as char) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some((start, end))
}

pub fn get_char_at_position(source: &str, line: u32, character: u32) -> Option<char> {
    let byte_idx = position_to_byte_offset(source, line, character)?;
    source.as_bytes().get(byte_idx).map(|&b| b as char)
}

#[cfg(test)]
mod tests {
    use super::word_byte_range;

    #[test]
    fn word_range_simple_name() {
        assert_eq!(word_byte_range("println", 0), Some((0, 7)));
    }

    #[test]
    fn word_range_in_dotted_path() {
        // cursor on 'p' of 'println' in "core.println"
        assert_eq!(word_byte_range("core.println", 5), Some((5, 12)));
    }

    #[test]
    fn word_range_root_of_dotted_path() {
        // cursor on 'c' of 'core' in "core.println"
        assert_eq!(word_byte_range("core.println", 0), Some((0, 4)));
    }

    #[test]
    fn word_range_on_dot_returns_none() {
        // cursor on the '.' in "core.println"
        assert_eq!(word_byte_range("core.println", 4), None);
    }

    #[test]
    fn word_range_multi_segment() {
        // cursor on 'b' in "a.b.c"
        assert_eq!(word_byte_range("a.b.c", 2), Some((2, 3)));
    }

    #[test]
    fn word_range_last_segment_multi() {
        // cursor on 'c' in "a.b.c"
        assert_eq!(word_byte_range("a.b.c", 4), Some((4, 5)));
    }

    #[test]
    fn word_range_with_underscore() {
        assert_eq!(word_byte_range("my_var", 0), Some((0, 6)));
    }

    #[test]
    fn word_range_non_identifier_returns_none() {
        // cursor on space in "use core"
        assert_eq!(word_byte_range("use core", 3), None);
    }

    #[test]
    fn word_range_out_of_bounds_returns_none() {
        assert_eq!(word_byte_range("abc", 100), None);
    }

    #[test]
    fn word_range_at_byte_after_word_returns_none() {
        // cursor right past the last character (byte_pos == len)
        assert_eq!(word_byte_range("abc", 3), None);
    }
}
