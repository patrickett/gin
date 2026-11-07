//! Position mapping: bytes and LSP's UTF-16 columns, with clamping instead of panics.
use ginlsp::position::LineIndex;
use lsp_types::Position;

fn at(line: u32, character: u32) -> Position {
    Position { line, character }
}

#[test]
fn utf16_columns_round_trip() {
    let text = "a\u{1F600}b\n\u{e9}x\r\nlast";
    let index = LineIndex::new(text);
    // The emoji is 4 bytes and 2 units.
    assert_eq!(index.position(text, 1), at(0, 1));
    assert_eq!(index.position(text, 5), at(0, 3));
    assert_eq!(index.offset(text, at(0, 3)), 5);
    assert_eq!(index.position(text, 7), at(1, 0));
    assert_eq!(index.position(text, 9), at(1, 1), "e-acute is 2 bytes, 1 unit");
    assert_eq!(index.offset(text, at(1, 1)), 9);
}

#[test]
fn out_of_range_positions_clamp() {
    let text = "ab\ncd";
    let index = LineIndex::new(text);
    assert_eq!(index.offset(text, at(0, 99)), 2, "end of line, before the newline");
    assert_eq!(index.offset(text, at(9, 0)), 5, "past the last line is the end");
    assert_eq!(index.position(text, 999), at(1, 2));
    // Inside an emoji (between its surrogates) rounds down to the character.
    let emoji = "\u{1F600}";
    let index = LineIndex::new(emoji);
    assert_eq!(index.offset(emoji, at(0, 1)), 4);
    assert_eq!(index.position(emoji, 2), at(0, 0), "a byte inside a scalar clamps back");
}

#[test]
fn crlf_lines_do_not_count_the_carriage_return() {
    let text = "ab\r\ncd";
    let index = LineIndex::new(text);
    assert_eq!(index.offset(text, at(0, 99)), 2);
    assert_eq!(index.position(text, 4), at(1, 0));
}
