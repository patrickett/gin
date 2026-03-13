use crate::util::is_identifier_char;
use tower_lsp::lsp_types::{Position, Range};

pub fn find_definition_line(source: &str, word: &str, is_tag: bool) -> Option<usize> {
    for (line_idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();

        if !trimmed.starts_with(word) {
            continue;
        }

        let after = &trimmed[word.len()..];
        if after.is_empty() {
            continue;
        }
        let next_ch = after.chars().next().unwrap();
        if is_identifier_char(next_ch) {
            continue;
        }

        if is_tag {
            let rest = after.trim_start();
            if rest.starts_with("has ") || rest.starts_with("is ") || rest.starts_with("::=") {
                return Some(line_idx);
            }
            if next_ch == '(' {
                if let Some(close) = rest.find(')') {
                    let after_params = rest[close + 1..].trim_start();
                    if after_params.starts_with("has ")
                        || after_params.starts_with("is ")
                        || after_params.starts_with("::=")
                    {
                        return Some(line_idx);
                    }
                }
            }
        } else {
            let rest = after.trim_start();
            if rest.starts_with(':') {
                return Some(line_idx);
            }
            if next_ch == '(' {
                if let Some(close) = rest.find(')') {
                    let after_params = rest[close + 1..].trim_start();
                    if after_params.starts_with(':') {
                        return Some(line_idx);
                    }
                }
            }
        }
    }
    None
}

pub fn find_definition_range(source: &str, word: &str, is_tag: bool) -> Range {
    if let Some(line_idx) = find_definition_line(source, word, is_tag) {
        let line = source.lines().nth(line_idx).unwrap_or("");
        let col = line.find(word).unwrap_or(0) as u32;
        return Range {
            start: Position {
                line: line_idx as u32,
                character: col,
            },
            end: Position {
                line: line_idx as u32,
                character: col + word.len() as u32,
            },
        };
    }
    Range::default()
}
