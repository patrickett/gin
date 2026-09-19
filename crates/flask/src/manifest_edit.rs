use std::ops::Range;

pub(crate) fn set_field(raw: &str, field: &str, value: &str) -> Option<String> {
    let tokens = tokens(raw)?;
    let mut depth = 0;
    for (i, range) in tokens.iter().enumerate() {
        match &raw[range.clone()] {
            "}" if depth == 1 => {
                let previous = &raw[tokens.get(i.checked_sub(1)?)?.clone()];
                let comma = if matches!(previous, "{" | ",") {
                    ""
                } else {
                    ","
                };
                return Some(format!(
                    "{}{comma}\n  {}: {value}\n{}",
                    &raw[..range.start],
                    serde_json::to_string(field).ok()?,
                    &raw[range.start..]
                ));
            }
            "{" | "[" => depth += 1,
            "}" | "]" => depth -= 1,
            key if depth == 1 => {
                let key = if key.starts_with(['\'', '"']) {
                    json5::from_str::<String>(key).ok()?
                } else {
                    key.to_string()
                };
                if key == field && tokens.get(i + 1).is_some_and(|r| &raw[r.clone()] == ":") {
                    let start = tokens.get(i + 2)?.start;
                    let mut nested = 0;
                    for token in &tokens[i + 2..] {
                        match &raw[token.clone()] {
                            "{" | "[" => nested += 1,
                            "}" | "]" => nested -= 1,
                            _ => {}
                        }
                        if nested == 0 {
                            return Some(format!("{}{value}{}", &raw[..start], &raw[token.end..]));
                        }
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

fn tokens(raw: &str) -> Option<Vec<Range<usize>>> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"/*") {
            i += 2;
            while bytes.get(i..i + 2) != Some(b"*/") {
                if i >= bytes.len() {
                    return None;
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        let start = i;
        match bytes[i] {
            quote @ (b'\'' | b'"') => {
                i += 1;
                while *bytes.get(i)? != quote {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => i += 1,
            _ => {
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b'{' | b'}' | b'[' | b']' | b':' | b',' | b'/')
                {
                    i += 1;
                }
                if i == start {
                    return None;
                }
            }
        }
        out.push(start..i);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_edit_preserves_comments_and_nested_versions() {
        let raw = "{ // version: 'fake'\n dependencies: {dep: {version: '^1'}}, /* keep */ version /* v */: '1.2.3', description: 'a \\\' quote' }";
        let updated = set_field(raw, "version", "\"1.3.0\"").unwrap();
        assert_eq!(updated, raw.replace("'1.2.3'", "\"1.3.0\""));
    }
}
