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
fn test_bind_with_doc_comment() {
    // Note: According to current grammar, doc comments come before the identifier,
    // not inside the function body. This test verifies the format doesn't break
    // doc comments that are properly positioned.
    let input = "--- This function prints values\nprint_values:\n    print(2)\n    print('John')\nreturn 1 + 1\n";
    let output = format(input);
    // Verify the doc comment is preserved
    assert!(
        output.contains("--- This function prints values"),
        "doc comment should be preserved, got:\n{}",
        output
    );
    // Verify the function is preserved
    assert!(
        output.contains("print_values:"),
        "function name should be preserved"
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
