use ast::source::{LineIndex, SourceExt};

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
