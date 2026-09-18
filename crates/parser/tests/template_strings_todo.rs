use ast::{BindValue, Expr, Literal};
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn interpolation_is_reserved_until_formatting_can_be_defined_in_gin() {
    for source in [
        "text: \"hello \\(name)\"\nnext: 1\n",
        "text := \"value \\(42)\"\nnext: 1\n",
        "text := \"\\(#target.triple)\"\nnext: 1\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.iter().any(|diagnostic| {
                diagnostic.code.slug() == "parse-template-strings-not-supported"
            }),
            "{:?}",
            parsed.symptoms
        );
        assert!(parsed.ast.defs.contains_key(&Intern::from_ref("next")));
    }
}

#[test]
fn quoted_text_without_interpolation_is_an_ordinary_byte_literal() {
    for (source, expected) in [
        ("text: \"hello\"\n", "hello"),
        ("text: \"\"\n", ""),
        ("text: \"hé\\0\\n\"\n", "hé\0\n"),
    ] {
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let bind = &parsed.ast.defs[&Intern::from_ref("text")];
        let BindValue::Expr(value) = &bind.value else {
            panic!("literal bind")
        };
        assert!(matches!(&value.value, Expr::Lit(Literal::String(text)) if text == expected));
    }
}

#[test]
fn template_allocation_is_not_a_compiler_query() {
    let parsed = "#template_memory(release)\nallocate: 0\n".parse_source_full();
    assert!(
        parsed
            .symptoms
            .iter()
            .any(|diagnostic| { diagnostic.code.slug() == "parse-unknown-compiler-query" })
    );
}
