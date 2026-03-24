use ginc::ast::ParameterKind;
use ropey::Rope;
use tower_lsp::lsp_types::Position;

pub fn get_word_at_position(source: &str, position: Position) -> Option<String> {
    let rope = Rope::from_str(source);
    let line = rope.try_line_to_char(position.line as usize).ok()?;
    let char_idx = line + position.character as usize;
    let byte_idx = rope.try_char_to_byte(char_idx).ok()?;

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

pub fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn format_params(params: &ginc::ast::Parameters) -> String {
    if params.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = params
        .iter()
        .map(|(name, kind)| match kind {
            ParameterKind::Generic => name.to_string(),
            ParameterKind::Tagged(tag) => format!("{name} {tag}"),
            ParameterKind::Default(expr) => format!("{name}: {expr:?}"),
        })
        .collect();
    format!("({})", parts.join(", "))
}

/// Extract a full numeric literal at `position`, including an optional leading `-`
/// and a decimal point (e.g. `-3`, `3.14`, `-0.5`).
/// Returns `None` if the cursor is not on a digit or a `-` immediately before digits.
pub fn get_number_at_position(source: &str, position: Position) -> Option<String> {
    let rope = Rope::from_str(source);
    let line = rope.try_line_to_char(position.line as usize).ok()?;
    let char_idx = line + position.character as usize;
    let byte_idx = rope.try_char_to_byte(char_idx).ok()?;

    let bytes = source.as_bytes();

    // Cursor must be on a digit, a dot that's part of a float, or a `-` before digits.
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

    // Walk left: include an optional leading `-`, then digits
    let mut start = byte_idx;
    while start > 0 && (bytes[start - 1] as char).is_ascii_digit() {
        start -= 1;
    }
    if start > 0 && bytes[start - 1] as char == '-' {
        start -= 1;
    }

    // Walk right: digits, optional `.digits`
    let mut end = byte_idx;
    while end < bytes.len() && (bytes[end] as char).is_ascii_digit() {
        end += 1;
    }
    if end < bytes.len() && bytes[end] as char == '.' {
        let after_dot = end + 1;
        if after_dot < bytes.len() && (bytes[after_dot] as char).is_ascii_digit() {
            end += 1; // consume the dot
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

pub fn get_char_at_position(source: &str, position: Position) -> Option<char> {
    let rope = Rope::from_str(source);
    let line = rope.try_line_to_char(position.line as usize).ok()?;
    let char_idx = line + position.character as usize;
    let byte_idx = rope.try_char_to_byte(char_idx).ok()?;
    source.as_bytes().get(byte_idx).map(|&b| b as char)
}

pub fn is_word_boundary(line: &str, pos: usize, len: usize) -> bool {
    let bytes = line.as_bytes();
    let before_ok = pos == 0 || !is_identifier_char(bytes[pos - 1] as char);
    let after_ok = pos + len >= bytes.len() || !is_identifier_char(bytes[pos + len] as char);
    before_ok && after_ok
}
