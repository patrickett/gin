//! Bounded `in` types and compile-time call-site checks.

mod support;

use diagnostic::Diagnostic;
use typecheck::FileId;
use typecheck::transform::transform_file;
const INT_TAG: &str = "Int is in 1...400\n\n";

fn with_int_tag(body: &str) -> String {
    format!("{INT_TAG}{body}")
}

fn collect_flaws(source: &str) -> Vec<Diagnostic> {
    let ast = parser::cursor::TokenCursor::parse_source(&with_int_tag(source));
    let typed = transform_file(ast, FileId(0));
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, f)| f.clone())
        .collect()
}

#[test]
fn bounded_param_accepts_literal_in_range() {
    let src = r#"
favorite_number(x in 0...10):
    favorite_number(9)
"#;
    let flaws = collect_flaws(src);
    assert!(
        !flaws.iter().any(|f| f.code.slug() == "type-out-of-range"),
        "expected no OutOfRange flaws, got {flaws:?}"
    );
}

#[test]
fn bounded_param_rejects_literal_out_of_range() {
    let src = r#"
favorite_number(x in 0...10):
    favorite_number(11)
"#;
    let flaws = collect_flaws(src);
    assert!(
        flaws.iter().any(|f| {
            f.code.slug() == "type-out-of-range"
                && f.arg("value") == Some("11")
                && f.arg("min") == Some("0")
                && f.arg("max") == Some("10")
        }),
        "expected OutOfRange for 11, got {flaws:?}"
    );
}

#[test]
fn in_on_bounded_int_tag_is_invalid() {
    let src = r#"
TinyInt is in 0...255

bad(n in TinyInt):
    return 0
"#;
    let ast = parser::cursor::TokenCursor::parse_source(&with_int_tag(src));
    let typed = transform_file(ast, FileId(0));
    assert!(
        typed
            .declaration_flaws
            .iter()
            .any(|(_, f)| f.code.slug() == "type-in-range-on-bounded-int-tag"),
        "expected InRangeOnBoundedIntTag flaw"
    );
}
