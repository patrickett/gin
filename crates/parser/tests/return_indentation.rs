use parser::cursor::TokenCursor;

#[test]
fn bind_return_must_align_with_bind_name() {
    let ast = TokenCursor::parse_source("bad() Int:\n    value: 7\n    return value\n");
    let errors: Vec<_> = ast
        .parse_warnings
        .iter()
        .filter(|d| d.code.slug() == "parse-indented-return")
        .collect();
    assert_eq!(errors.len(), 1, "{:?}", ast.parse_warnings);
    assert_eq!(errors[0].category, diagnostic::Category::Flaw);
    assert_eq!(
        errors[0].message,
        "return must align with the bind name; dedent it to close the bind"
    );
}

#[test]
fn aligned_bind_return_is_accepted() {
    let ast = TokenCursor::parse_source("good() Int:\n    value: 7\nreturn value\n");
    assert!(ast.parse_warnings.is_empty(), "{:?}", ast.parse_warnings);
}

#[test]
fn nested_bind_return_aligns_with_nested_name() {
    let ast = TokenCursor::parse_source(
        "outer() Int:\n    inner() Int:\n        value: 7\n    return value\nreturn inner()\n",
    );
    assert!(ast.parse_warnings.is_empty(), "{:?}", ast.parse_warnings);
}

#[test]
fn malformed_return_recovers_before_next_bind() {
    let ast = TokenCursor::parse_source(
        "bad() Int:\n    value: 7\n    return value\ngood() Int:\n    value: 8\nreturn value\n",
    );
    let codes: Vec<_> = ast.parse_warnings.iter().map(|d| d.code.slug()).collect();
    assert_eq!(codes, ["parse-indented-return"]);
}

#[test]
fn inline_return_is_accepted() {
    let ast = TokenCursor::parse_source("good() Int: return 7\n");
    assert!(ast.parse_warnings.is_empty(), "{:?}", ast.parse_warnings);
}

#[test]
fn indented_if_return_is_an_error() {
    let ast = TokenCursor::parse_source(
        "bad(flag Int) Int:\n    if flag is 1\n        return 7\n    return 8\nreturn 9\n",
    );
    let error = ast
        .parse_warnings
        .iter()
        .find(|d| d.code.slug() == "parse-indented-return")
        .expect("indentation diagnostic");
    assert_eq!(error.category, diagnostic::Category::Flaw);
}

#[test]
fn full_parse_reports_the_return_keyword_as_a_flaw() {
    use parser::query::SourceParseExt;
    let source = "bad() Int:\n    value: 7\n    return value\n";
    let output = source.parse_source_full();
    let error = output
        .symptoms
        .iter()
        .find(|d| d.code.slug() == "parse-indented-return")
        .expect("indentation diagnostic");
    assert_eq!(error.category, diagnostic::Category::Flaw);
    assert_eq!(&source[error.span.start()..error.span.end()], "return");
}
