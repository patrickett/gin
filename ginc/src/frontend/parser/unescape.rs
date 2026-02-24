/// support `\n`, `\t`, `\r`, `\\`, `\'`, `\"`, `\0`, `\(`.
pub fn unescape(raw: &str) -> String {
    // Fast path: no backslashes → return as-is
    if !raw.contains('\\') {
        return raw.to_owned();
    }

    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('0') => out.push('\0'),
            Some('(') => out.push('('),
            Some(other) => {
                // Unknown escape just pass through literally
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_escapes() {
        assert_eq!(unescape("hello world"), "hello world");
    }

    #[test]
    fn test_newline() {
        assert_eq!(unescape("hello\\nworld"), "hello\nworld");
    }

    #[test]
    fn test_tab() {
        assert_eq!(unescape("tab\\there"), "tab\there");
    }

    #[test]
    fn test_backslash() {
        assert_eq!(unescape("back\\\\slash"), "back\\slash");
    }

    #[test]
    fn test_null() {
        assert_eq!(unescape("null\\0byte"), "null\0byte");
    }

    #[test]
    fn test_quotes() {
        assert_eq!(unescape("say\\'hi\\'"), "say'hi'");
        assert_eq!(unescape("say\\\"hi\\\""), "say\"hi\"");
    }

    #[test]
    fn test_escaped_paren() {
        assert_eq!(unescape("\\(not interp)"), "(not interp)");
    }

    #[test]
    fn test_unknown_escape_passthrough() {
        assert_eq!(unescape("\\q"), "\\q");
    }

    #[test]
    fn test_trailing_backslash() {
        assert_eq!(unescape("end\\"), "end\\");
    }

    #[test]
    fn test_multiple_escapes() {
        assert_eq!(unescape("a\\nb\\tc\\\\d"), "a\nb\tc\\d");
    }
}
