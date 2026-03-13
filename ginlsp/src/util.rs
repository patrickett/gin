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
            ParameterKind::Tagged(tag) => format!("{name} {tag:?}"),
            ParameterKind::Default(expr) => format!("{name}: {expr:?}"),
        })
        .collect();
    format!("({})", parts.join(", "))
}

pub fn is_word_boundary(line: &str, pos: usize, len: usize) -> bool {
    let bytes = line.as_bytes();
    let before_ok = pos == 0 || !is_identifier_char(bytes[pos - 1] as char);
    let after_ok = pos + len >= bytes.len() || !is_identifier_char(bytes[pos + len] as char);
    before_ok && after_ok
}
