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
    crate::is_comment_at(source, byte_pos)
}

pub fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn get_word_at_position(source: &str, line: u32, character: u32) -> Option<String> {
    let byte_idx = position_to_byte_offset(source, line, character)?;

    let mut start = byte_idx;
    let mut end = byte_idx;

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

    Some(source[start..end].to_string())
}

pub fn get_char_at_position(source: &str, line: u32, character: u32) -> Option<char> {
    let byte_idx = position_to_byte_offset(source, line, character)?;
    source.as_bytes().get(byte_idx).map(|&b| b as char)
}
