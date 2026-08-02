//! Regression: union variants with postfix `---` doc comments on the same line,
//! continuation `or` on an indented line, and no doc on the declare itself.

use ast::DeclareValue;
use parser::query::SourceParseExt;

mod support;
use support::*;

#[test]
fn parse_infection_union_minimal_inline_fixture() {
    let src = "\
Infection is AnyField  --- AND-like
          or AllFields --- OR-like
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let infection = out.ast.tags.get(&intern("Infection")).expect("Infection");
    assert!(infection.doc_comment.is_none());

    let DeclareValue::Union { variants } = &infection.value else {
        panic!("expected union");
    };
    assert_eq!(variants.len(), 2);
    assert_eq!(variant_name(&variants[0]), "AnyField");
    assert_eq!(variant_name(&variants[1]), "AllFields");
    assert_eq!(variant_doc(&variants[0]), Some("AND-like"));
    assert_eq!(variant_doc(&variants[1]), Some("OR-like"));
}
