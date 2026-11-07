//! Byte offsets (what `ast` and the checker speak) and LSP positions (zero-based line,
//! UTF-16 code units in the line). Out-of-range positions clamp, never panic.

use lsp_types::{Position, Range};

/// Byte offsets of the start of each line of one text.
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
        Self { starts, len: text.len() }
    }

    /// The position of `byte`, clamped to the text and to a character boundary.
    pub fn position(&self, text: &str, byte: usize) -> Position {
        let mut byte = byte.min(self.len);
        while !text.is_char_boundary(byte) {
            byte -= 1;
        }
        let line = self.starts.partition_point(|start| *start <= byte) - 1;
        let character = text[self.starts[line]..byte].encode_utf16().count();
        Position { line: u32::try_from(line).unwrap_or(u32::MAX), character: u32::try_from(character).unwrap_or(u32::MAX) }
    }

    pub fn range(&self, text: &str, start: usize, end: usize) -> Range {
        Range { start: self.position(text, start), end: self.position(text, end.max(start)) }
    }

    /// The byte offset of `position`. A line past the end is the end of the text; a character
    /// past the end of its line is the end of that line (before its newline); one inside a
    /// surrogate pair rounds down to the character.
    pub fn offset(&self, text: &str, position: Position) -> usize {
        let line = position.line as usize;
        let Some(&start) = self.starts.get(line) else { return self.len };
        let end = self.starts.get(line + 1).map_or(self.len, |next| next - 1);
        let line_text = text[start..end].trim_end_matches('\r');
        let mut units = 0u32;
        for (index, c) in line_text.char_indices() {
            if units >= position.character {
                return start + index;
            }
            units += c.len_utf16() as u32;
        }
        start + line_text.len()
    }
}
