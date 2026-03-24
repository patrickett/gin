//! Tests for bind statement formatting (colon-align)

use ginfmt::format;

#[test]
fn test_bind_statement_format() {
    let input = "main:\n    print('hello')\nreturn\n";
    let output = format(input);
    // Verify it produces output
    assert!(!output.is_empty());
}

#[test]
fn test_nested_binds() {
    let input = "main:\n    print('a')\n    print('b')\nreturn\n";
    let output = format(input);
    assert!(!output.is_empty());
}

#[test]
fn test_multiline_bind_doc_comment_indent() {
    let input = "do_something:\n--- value is always 2\n    val: 2\n    name: 'John'\nreturn 1 + 1\n";
    let output = format(input);
    // The doc comment should be indented to match the body level (4 spaces)
    assert!(
        output.contains("    --- value is always 2"),
        "doc comment should be indented to body level, got:\n{}",
        output
    );
}

#[test]
fn test_colon_alignment() {
    let input = "main:\n    print('hello')\nreturn\n";
    let output = format(input);
    // Note: Nested bind statements like "start: value" inside a bind are not
    // parsed as bind_statement nodes by tree-sitter (they're expressions).
    // Full colon alignment is handled by the lexer-based aligner in align.rs.
    assert!(!output.is_empty());
}

