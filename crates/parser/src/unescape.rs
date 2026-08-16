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
#[path = "../tests/unescape_tests.rs"]
mod tests;
