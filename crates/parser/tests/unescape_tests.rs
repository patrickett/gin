use parser::unescape::UnescapeExt;

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
