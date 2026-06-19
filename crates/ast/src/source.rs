//! Source code utilities for LSP (position conversion, word extraction, etc.)

/// Pre-computed line-start offsets for fast LSP position conversion.
///
/// Construction is O(n) but then each `byte_to_position` is O(log n + Σ line),
/// where Σ line is a UTF-16 scan of a single line. This is far faster than
/// `SourceExt::byte_offset_to_position` (which scans from byte 0) when
/// positions are near the end of large files.
///
/// All positions use 0-based (line, column) where column is measured in UTF-16
/// code units, matching the LSP specification.
///
/// The index does **not** own the source text — you must pass the source
/// as `&str` when calling conversion methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    /// Byte offset of each line start (position 0 is always included).
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Build a line index from the given source text.
    ///
    /// Time: O(n) where n is the number of bytes in source.
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    /// Return the number of lines in the source (1 + newline count).
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Convert a byte offset to a 0-based (line, column) position.
    ///
    /// Column is measured in UTF-16 code units (LSP specification).
    ///
    /// Time: O(log n + m), where n = lines, m = UTF-16 scanning of the
    ///       target line.
    pub fn byte_to_position(&self, source: &str, byte: usize) -> (u32, u32) {
        debug_assert!(
            byte <= source.len(),
            "byte {byte} out of bounds (len {})",
            source.len()
        );
        let byte = byte.min(source.len());

        // Binary search to find which line this byte belongs to.
        let line_idx = match self.line_starts.binary_search(&byte) {
            Ok(line) => line,      // exactly at a line start
            Err(0) => 0,           // before first line start → line 0
            Err(next) => next - 1, // in middle of line `next - 1`
        };

        let line_start = self.line_starts[line_idx];

        // Count UTF-16 code units from line_start to byte.
        let col = source[line_start..byte]
            .chars()
            .map(|ch| ch.len_utf16() as u32)
            .sum();

        (line_idx as u32, col)
    }

    /// Convert a (line, column) position to a byte offset.
    ///
    /// Column is interpreted as UTF-16 code units (LSP specification).
    /// Returns `None` if the line does not exist or the column is invalid.
    ///
    /// Time: O(m), where m = UTF-16 scanning of the target line.
    pub fn position_to_byte(&self, source: &str, line: u32, character: u32) -> Option<usize> {
        let line = line as usize;
        let line_start = *self.line_starts.get(line)?;

        let line_end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(source.len());

        let line_text = &source[line_start..line_end];
        let mut utf16_units: u32 = 0;

        for (byte_idx, ch) in line_text.char_indices() {
            if utf16_units == character {
                return Some(line_start + byte_idx);
            }
            utf16_units += ch.len_utf16() as u32;
            if utf16_units > character {
                // Mid-surrogate or multi-unit character overshoot.
                return None;
            }
        }

        // Character may point past the last char but within the line
        // (equivalent to end-of-line position).
        if utf16_units == character {
            Some(line_end)
        } else {
            None
        }
    }

    /// Convert a byte range into an LSP-compatible (start, end) position pair.
    pub fn byte_range_to_positions(
        &self,
        source: &str,
        start: usize,
        end: usize,
    ) -> ((u32, u32), (u32, u32)) {
        (
            self.byte_to_position(source, start),
            self.byte_to_position(source, end),
        )
    }
}

/// Extension trait providing source-text utility methods on `str`.
pub trait SourceExt {
    /// Return the byte offset of each line start (0-indexed byte positions,
    /// with position 0 always included as the first line start).
    fn compute_line_starts(&self) -> Vec<usize>;

    /// Convert a byte offset to (line, column) position.
    ///
    /// Column is measured in UTF-16 code units (LSP specification requirement).
    fn byte_offset_to_position(&self, byte: usize) -> (u32, u32);

    /// Convert a (line, column) position to a byte offset.
    ///
    /// Column is interpreted as UTF-16 code units (LSP specification).
    fn position_to_byte_offset(&self, line: u32, character: u32) -> Option<usize>;

    /// Check whether the given (line, character) position falls inside a `--` line comment.
    fn is_in_comment(&self, line: u32, character: u32) -> bool;

    /// Extract the identifier word at `byte_pos`.
    fn word_at_byte_offset(&self, byte_pos: usize) -> Option<String>;

    /// Identifier at `byte_pos`, or the unquoted contents of a single-quoted literal.
    fn symbol_at_byte_offset(&self, byte_pos: usize) -> Option<String>;

    /// Return the (start, end) byte range of the identifier word at `byte_pos`.
    /// Returns `None` when the cursor is not on an identifier character.
    fn word_byte_range(&self, byte_pos: usize) -> Option<(usize, usize)>;

    /// Get the character at a (line, character) position.
    fn get_char_at_position(&self, line: u32, character: u32) -> Option<char>;

    /// Check whether `byte_pos` falls on a line that already has `--` before it.
    fn is_comment_at(&self, byte_pos: usize) -> bool;

    /// `byte_pos` is inside `'…'` (including the quote characters).
    fn single_quoted_literal_range(&self, byte_pos: usize) -> Option<(usize, usize)>;
}

impl SourceExt for str {
    /// Check whether `byte_pos` falls on a line that already has `--` before it.
    fn is_comment_at(&self, byte_pos: usize) -> bool {
        let line_start = self[..byte_pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = &self[line_start..];
        if let Some(comment_start) = line.find("--") {
            let comment_byte = line_start + comment_start;
            byte_pos >= comment_byte
        } else {
            false
        }
    }

    /// `byte_pos` is inside `'…'` (including the quote characters).
    fn single_quoted_literal_range(&self, byte_pos: usize) -> Option<(usize, usize)> {
        if byte_pos >= self.len() {
            return None;
        }
        let bytes = self.as_bytes();
        let mut start = byte_pos;
        if bytes[start] != b'\'' {
            while start > 0 && bytes[start] != b'\'' {
                start -= 1;
            }
            if start == 0 && bytes[start] != b'\'' {
                return None;
            }
        }
        if bytes[start] != b'\'' {
            return None;
        }
        let mut end = start + 1;
        while end < bytes.len() && bytes[end] != b'\'' {
            end += 1;
        }
        if end >= bytes.len() || bytes[end] != b'\'' {
            return None;
        }
        end += 1;
        (byte_pos >= start && byte_pos < end).then_some((start, end))
    }

    fn compute_line_starts(&self) -> Vec<usize> {
        let mut starts = vec![0];
        for (i, b) in self.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        starts
    }

    fn byte_offset_to_position(&self, byte: usize) -> (u32, u32) {
        let mut line = 0u32;
        let mut col = 0u32;
        let mut current_byte = 0usize;

        for ch in self.chars() {
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

    fn position_to_byte_offset(&self, line: u32, character: u32) -> Option<usize> {
        let line_start: usize = self
            .split('\n')
            .take(line as usize)
            .map(|l| l.len() + 1)
            .sum();
        if line_start > self.len() {
            return None;
        }
        let mut utf16_units = 0u32;
        for (byte_idx, c) in self[line_start..].char_indices() {
            if utf16_units == character {
                return Some(line_start + byte_idx);
            }
            utf16_units += c.len_utf16() as u32;
        }
        (utf16_units == character).then_some(line_start + self[line_start..].len())
    }

    fn is_in_comment(&self, line: u32, character: u32) -> bool {
        let Some(byte_pos) = self.position_to_byte_offset(line, character) else {
            return false;
        };
        self.is_comment_at(byte_pos)
    }

    fn word_at_byte_offset(&self, byte_pos: usize) -> Option<String> {
        let (start, end) = self.word_byte_range(byte_pos)?;
        Some(self[start..end].to_string())
    }

    fn symbol_at_byte_offset(&self, byte_pos: usize) -> Option<String> {
        if let Some(word) = self.word_at_byte_offset(byte_pos) {
            return Some(word);
        }
        let (start, end) = self.single_quoted_literal_range(byte_pos)?;
        let inner = &self[start + 1..end - 1];
        (!inner.is_empty()).then(|| inner.to_string())
    }

    fn word_byte_range(&self, byte_pos: usize) -> Option<(usize, usize)> {
        let bytes = self.as_bytes();
        if byte_pos >= bytes.len() || !(bytes[byte_pos] as char).is_identifier() {
            return None;
        }
        let mut start = byte_pos;
        let mut end = byte_pos;
        while start > 0 && (bytes[start - 1] as char).is_identifier() {
            start -= 1;
        }
        while end < bytes.len() && (bytes[end] as char).is_identifier() {
            end += 1;
        }
        if start == end {
            return None;
        }
        Some((start, end))
    }

    fn get_char_at_position(&self, line: u32, character: u32) -> Option<char> {
        let byte_idx = self.position_to_byte_offset(line, character)?;
        self.as_bytes().get(byte_idx).map(|&b| b as char)
    }
}

/// Extension trait providing [`byte_offset_to_line_col`] on `[usize]`.
pub trait LineStartsExt {
    /// Convert a byte offset to 1-based (line, column) using pre-computed line starts.
    ///
    /// Columns are measured in bytes (not UTF-16 code units).
    fn byte_offset_to_line_col(&self, byte: usize) -> (usize, usize);
}

impl LineStartsExt for [usize] {
    fn byte_offset_to_line_col(&self, byte: usize) -> (usize, usize) {
        if self.is_empty() {
            return (1, 1);
        }
        let line_idx = self.partition_point(|&s| s <= byte);
        let line_idx = line_idx.saturating_sub(1).min(self.len().saturating_sub(1));
        let line_start = self[line_idx];
        let line_no = line_idx + 1;
        let col = byte.saturating_sub(line_start) + 1;
        (line_no, col)
    }
}

/// Extension trait providing [`is_identifier`](CharExt::is_identifier) on `char`.
#[allow(clippy::wrong_self_convention)]
pub trait CharExt {
    /// Check if this character is a valid Gin identifier character.
    fn is_identifier(self) -> bool;
}

impl CharExt for char {
    fn is_identifier(self) -> bool {
        self.is_alphanumeric() || self == '_'
    }
}

#[cfg(test)]
mod tests {
    use super::{LineIndex, SourceExt};

    #[test]
    fn line_index_empty_source() {
        let idx = LineIndex::new("");
        assert_eq!(idx.line_count(), 1);
        assert_eq!(idx.byte_to_position("", 0), (0, 0));
        assert_eq!(idx.position_to_byte("", 0, 0), Some(0));
        assert_eq!(idx.position_to_byte("", 1, 0), None);
    }

    #[test]
    fn line_index_single_line_ascii() {
        let source = "hello";
        let idx = LineIndex::new(source);
        assert_eq!(idx.line_count(), 1);
        assert_eq!(idx.byte_to_position(source, 0), (0, 0));
        assert_eq!(idx.byte_to_position(source, 1), (0, 1));
        assert_eq!(idx.byte_to_position(source, 5), (0, 5));
        assert_eq!(idx.position_to_byte(source, 0, 0), Some(0));
        assert_eq!(idx.position_to_byte(source, 0, 5), Some(5));
    }

    #[test]
    fn line_index_multi_line_ascii() {
        let source = "abc\ndef\nghi";
        let idx = LineIndex::new(source);
        assert_eq!(idx.line_count(), 3);

        // Line 0
        assert_eq!(idx.byte_to_position(source, 0), (0, 0));
        assert_eq!(idx.byte_to_position(source, 1), (0, 1));
        assert_eq!(idx.byte_to_position(source, 2), (0, 2));
        assert_eq!(idx.byte_to_position(source, 3), (0, 3));

        // Line 1
        assert_eq!(idx.byte_to_position(source, 4), (1, 0));
        assert_eq!(idx.byte_to_position(source, 6), (1, 2));

        // Line 2
        assert_eq!(idx.byte_to_position(source, 8), (2, 0));
        assert_eq!(idx.byte_to_position(source, 10), (2, 2));

        // position_to_byte round-trips
        assert_eq!(idx.position_to_byte(source, 0, 0), Some(0));
        assert_eq!(idx.position_to_byte(source, 0, 3), Some(3));
        assert_eq!(idx.position_to_byte(source, 1, 0), Some(4));
        assert_eq!(idx.position_to_byte(source, 1, 2), Some(6));
        assert_eq!(idx.position_to_byte(source, 2, 2), Some(10));
    }

    #[test]
    fn line_index_at_newline_byte() {
        // Byte at the exact newline position belongs to the line before it.
        let source = "ab\nc";
        let idx = LineIndex::new(source);
        // The newline at byte 2 ends line 0. The char '\n' itself contributes to
        // line 0's column (it's 1 UTF-16 unit).
        assert_eq!(idx.byte_to_position(source, 2), (0, 2));
        // Byte 3 starts line 1
        assert_eq!(idx.byte_to_position(source, 3), (1, 0));
    }

    #[test]
    fn line_index_position_to_byte_out_of_bounds() {
        let source = "hello";
        let idx = LineIndex::new(source);
        assert_eq!(idx.position_to_byte(source, 0, 99), None);
        assert_eq!(idx.position_to_byte(source, 999, 0), None);
    }

    #[test]
    fn line_index_round_trip() {
        let source = "a\nbc\ndef\n";
        let idx = LineIndex::new(source);
        // Test many offsets round-trip.
        for byte in 0..source.len() {
            let (line, col) = idx.byte_to_position(source, byte);
            let round = idx.position_to_byte(source, line, col);
            assert_eq!(
                round,
                Some(byte),
                "round-trip failed at byte {byte} (line={line}, col={col})"
            );
        }
    }

    #[test]
    fn line_index_utf8_multi_byte_bmp() {
        // é is 2 bytes in UTF-8, 1 UTF-16 unit.
        let source = "héllo";
        let idx = LineIndex::new(source);
        assert_eq!(idx.line_count(), 1);
        // source = "héllo"
        // bytes:  h(0x68=1) é(C3 A9=2) l(6C=1) l(6C=1) o(6F=1) = 6
        // chars:  h, é, l, l, o = 5
        // UTF-16: h(1) é(1) l(1) l(1) o(1) = 5 code units
        //
        // Char boundaries only: 0, 1, 3, 4, 5, 6.
        // Byte 2 is in the middle of é — not a valid input.

        // byte_to_position only valid at char boundaries:
        assert_eq!(idx.byte_to_position(source, 0), (0, 0)); // 'h', col 0
        assert_eq!(idx.byte_to_position(source, 1), (0, 1)); // start of é, col 0+1
        assert_eq!(idx.byte_to_position(source, 3), (0, 2)); // 'l', col 2 (h+é=2)
        assert_eq!(idx.byte_to_position(source, 4), (0, 3)); // 'l', col 3
        assert_eq!(idx.byte_to_position(source, 5), (0, 4)); // 'o', col 4
        assert_eq!(idx.byte_to_position(source, 6), (0, 5)); // end, col 5

        // position_to_byte
        assert_eq!(idx.position_to_byte(source, 0, 0), Some(0)); // h
        assert_eq!(idx.position_to_byte(source, 0, 1), Some(1)); // start of é
        assert_eq!(idx.position_to_byte(source, 0, 2), Some(3)); // start of l
        assert_eq!(idx.position_to_byte(source, 0, 3), Some(4)); // start of l
        assert_eq!(idx.position_to_byte(source, 0, 4), Some(5)); // start of o
        assert_eq!(idx.position_to_byte(source, 0, 5), Some(6)); // end of o

        // col past end of line -> None
        assert_eq!(idx.position_to_byte(source, 0, 99), None);
    }

    #[test]
    fn line_index_emoji_surrogate_pairs() {
        // 😀 is 4 bytes in UTF-8, 2 UTF-16 units (surrogate pair).
        let source = "a😀b";
        let idx = LineIndex::new(source);
        // Bytes:
        //   a = 0x61 (byte 0)
        //   😀 = F0 9F 98 80 (bytes 1-4)
        //   b = 0x62 (byte 5)
        //
        // UTF-16 code units:
        //   a = 1 unit
        //   😀 = 2 units
        //   b = 1 unit
        //   total: 4 units

        // byte_to_position
        assert_eq!(idx.byte_to_position(source, 0), (0, 0)); // 'a'
        assert_eq!(idx.byte_to_position(source, 1), (0, 1)); // start of 😀
        assert_eq!(idx.byte_to_position(source, 5), (0, 3)); // 'b' (after 😀's 2 units)
        assert_eq!(idx.byte_to_position(source, 6), (0, 4)); // end

        // position_to_byte
        assert_eq!(idx.position_to_byte(source, 0, 0), Some(0)); // 'a'
        assert_eq!(idx.position_to_byte(source, 0, 1), Some(1)); // start of 😀
        // Column 2 is in the middle of the surrogate pair -> technically invalid.
        // LSP positions should never be in the middle of a surrogate pair.
        assert_eq!(idx.position_to_byte(source, 0, 2), None);
        assert_eq!(idx.position_to_byte(source, 0, 3), Some(5)); // 'b'
        assert_eq!(idx.position_to_byte(source, 0, 4), Some(6)); // end
    }

    #[test]
    fn line_index_unicode_round_trip() {
        let source = "héllo\nw😀rld\n💯\n";
        let idx = LineIndex::new(source);
        // Only step through char boundaries (byte_to_position requires valid
        // char-boundary inputs).
        let mut byte = 0;
        while byte < source.len() {
            let (line, col) = idx.byte_to_position(source, byte);
            let round = idx.position_to_byte(source, line, col);
            assert_eq!(
                round,
                Some(byte),
                "round-trip failed at byte {byte} (line={line}, col={col})"
            );
            byte += 1;
            while byte < source.len() && !source.is_char_boundary(byte) {
                byte += 1;
            }
        }
    }

    #[test]
    fn line_index_byte_range_to_positions() {
        let source = "abc\ndef\nghi";
        let idx = LineIndex::new(source);
        let (start, end) = idx.byte_range_to_positions(source, 4, 7);
        assert_eq!(start, (1, 0)); // 'd'
        assert_eq!(end, (1, 3)); // 'f' (after 'def')
    }

    #[test]
    fn line_index_consistency_with_sourecext() {
        let source = "a\nhello world\nfoo\nbar baz\n";
        let idx = LineIndex::new(source);
        for byte in 0..source.len() {
            let (line_ext, col_ext) = source.byte_offset_to_position(byte);
            let (line_idx, col_idx) = idx.byte_to_position(source, byte);
            assert_eq!(
                (line_ext, col_ext),
                (line_idx, col_idx),
                "mismatch at byte {byte}"
            );
        }
    }

    #[test]
    fn line_index_position_to_byte_col_past_line_end_returns_none() {
        let source = "a\nhello world\nfoo\nbar baz\n";
        let idx = LineIndex::new(source);
        // Line 0 = "a\n" which is 2 UTF-16 code units ('a' = 1, '\n' = 1).
        // col 0 -> byte 0, col 1 -> byte 1, col 2 -> byte 2 (= end of line).
        assert_eq!(idx.position_to_byte(source, 0, 0), Some(0));
        assert_eq!(idx.position_to_byte(source, 0, 1), Some(1));
        assert_eq!(idx.position_to_byte(source, 0, 2), Some(2)); // end of line
        assert_eq!(idx.position_to_byte(source, 0, 3), None);

        // Line 1 = "hello world\n" = 12 UTF-16 code units.
        assert_eq!(idx.position_to_byte(source, 1, 0), Some(2));
        assert_eq!(idx.position_to_byte(source, 1, 11), Some(13)); // 'd'
        assert_eq!(idx.position_to_byte(source, 1, 12), Some(14)); // end (newline)
        assert_eq!(idx.position_to_byte(source, 1, 13), None);
    }

    #[test]
    fn line_index_large_file_no_panic() {
        // A source large enough to trigger binary search edge cases.
        let lines: Vec<String> = (0..10000).map(|i| format!("line_{i}\n")).collect();
        let source = lines.concat();
        let idx = LineIndex::new(&source);
        // 10000 lines = 10000 newlines + 1 (the implicit start) = 10001 line starts
        assert_eq!(idx.line_count(), 10001);

        // Quick check on first and last line.
        assert_eq!(idx.byte_to_position(&source, 0), (0, 0));
        let last_byte = source.len().saturating_sub(1);
        let (_line, _col) = idx.byte_to_position(&source, last_byte);

        // Round-trip consistency on a few positions.
        for byte in [0, 100, 1000, 10000, 50000, source.len() / 2, last_byte] {
            if byte <= source.len() && source.is_char_boundary(byte) {
                let (l, c) = idx.byte_to_position(&source, byte);
                assert_eq!(idx.position_to_byte(&source, l, c), Some(byte));
            }
        }
    }

    #[test]
    fn word_range_simple_name() {
        assert_eq!("println".word_byte_range(0), Some((0, 7)));
    }

    #[test]
    fn word_range_in_dotted_path() {
        // cursor on 'p' of 'println' in "core.println"
        assert_eq!("core.println".word_byte_range(5), Some((5, 12)));
    }

    #[test]
    fn word_range_root_of_dotted_path() {
        // cursor on 'c' of 'core' in "core.println"
        assert_eq!("core.println".word_byte_range(0), Some((0, 4)));
    }

    #[test]
    fn word_range_on_dot_returns_none() {
        // cursor on the '.' in "core.println"
        assert_eq!("core.println".word_byte_range(4), None);
    }

    #[test]
    fn word_range_multi_segment() {
        // cursor on 'b' in "a.b.c"
        assert_eq!("a.b.c".word_byte_range(2), Some((2, 3)));
    }

    #[test]
    fn word_range_last_segment_multi() {
        // cursor on 'c' in "a.b.c"
        assert_eq!("a.b.c".word_byte_range(4), Some((4, 5)));
    }

    #[test]
    fn word_range_with_underscore() {
        assert_eq!("my_var".word_byte_range(0), Some((0, 6)));
    }

    #[test]
    fn word_range_non_identifier_returns_none() {
        // cursor on space in "use core"
        assert_eq!("use core".word_byte_range(3), None);
    }

    #[test]
    fn word_range_out_of_bounds_returns_none() {
        assert_eq!("abc".word_byte_range(100), None);
    }

    #[test]
    fn word_range_at_byte_after_word_returns_none() {
        // cursor right past the last character (byte_pos == len)
        assert_eq!("abc".word_byte_range(3), None);
    }
}
