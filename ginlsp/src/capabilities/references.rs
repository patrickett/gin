use crate::util::is_word_boundary;
use tower_lsp::lsp_types::{Location, Position, Range, Url};

pub fn find_all_references(source: &str, word: &str, uri: &Url) -> Vec<Location> {
    let mut locations = Vec::new();

    for (line_idx, line) in source.lines().enumerate() {
        let mut col = 0;
        while let Some(pos) = line[col..].find(word) {
            let abs_col = col + pos;
            if is_word_boundary(line, abs_col, word.len()) {
                locations.push(Location {
                    uri: uri.clone(),
                    range: Range {
                        start: Position {
                            line: line_idx as u32,
                            character: abs_col as u32,
                        },
                        end: Position {
                            line: line_idx as u32,
                            character: (abs_col + word.len()) as u32,
                        },
                    },
                });
            }
            col = abs_col + word.len();
        }
    }

    locations
}
