//! Regression: `--` doc-comment lines inside a bind body must not make the parser think
//! `return` is missing.

use parser::parse_source_full;

#[test]
fn bind_body_skips_doc_comments_before_return() {
    let source = r"use core

main:
    cor
    -- for i in 0..10
        -- io.println('Hello, world!')
    -- loop
return
";
    let out = parse_source_full(source);
    let bogus: Vec<_> = out
        .symptoms
        .iter()
        .filter(|s| s.message.contains("expected 'return' after bind body"))
        .collect();
    assert!(
        bogus.is_empty(),
        "unexpected bind-body return parse errors: {:?}",
        bogus
    );
}
