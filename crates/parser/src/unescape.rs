/// Extension trait providing string escape utilities on `str`.
pub trait UnescapeExt {
    /// Unescape common escape sequences (`\n`, `\t`, `\r`, `\\`, `\'`, `\"`, `\0`, `\(`).
    /// Unknown escapes pass through literally.
    fn unescape(&self) -> String;
}

impl UnescapeExt for str {
    fn unescape(&self) -> String {
        // Fast path: no backslashes → return as-is
        if !self.contains('\\') {
            return self.to_owned();
        }

        let mut out = String::with_capacity(self.len());
        let mut chars = self.chars();

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_escapes() {
        assert_eq!("hello world".unescape(), "hello world");
    }

    #[test]
    fn test_newline() {
        assert_eq!("hello\\nworld".unescape(), "hello\nworld");
    }

    #[test]
    fn test_tab() {
        assert_eq!("tab\\there".unescape(), "tab\there");
    }

    #[test]
    fn test_backslash() {
        assert_eq!("back\\\\slash".unescape(), "back\\slash");
    }

    #[test]
    fn test_null() {
        assert_eq!("null\\0byte".unescape(), "null\0byte");
    }

    #[test]
    fn test_quotes() {
        assert_eq!("say\\'hi\\'".unescape(), "say'hi'");
        assert_eq!("say\\\"hi\\\"".unescape(), "say\"hi\"");
    }

    #[test]
    fn test_escaped_paren() {
        assert_eq!("\\(not interp)".unescape(), "(not interp)");
    }

    #[test]
    fn test_unknown_escape_passthrough() {
        assert_eq!("\\q".unescape(), "\\q");
    }

    #[test]
    fn test_trailing_backslash() {
        assert_eq!("end\\".unescape(), "end\\");
    }

    #[test]
    fn test_multiple_escapes() {
        assert_eq!("a\\nb\\tc\\\\d".unescape(), "a\nb\tc\\d");
    }
}
