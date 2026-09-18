use parser::query::SourceParseExt;

#[test]
fn nested_type_application_can_precede_a_constant_argument() {
    let source = "Holder has values Array(Slice(Byte), 2)\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
}

#[test]
fn text_literal_is_not_a_compiler_query() {
    let parsed = "#text_literal\nSlice(element) has ptr @element\n".parse_source_full();
    assert!(
        parsed
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "parse-unknown-compiler-query"),
        "{:?}",
        parsed.symptoms
    );
}
