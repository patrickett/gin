#[test]
fn test_sources_parse_cleanly() {
    let ast = parser::cursor::TokenCursor::parse_source("Maybe(x) is\n    Some(x) or\n    None");
    assert!(!ast.defs.is_empty() || !ast.tags.is_empty() || !ast.exprs.is_empty());
}
