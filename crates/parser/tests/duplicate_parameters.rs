use parser::query::SourceParseExt;

#[test]
fn parameters_cannot_redeclare_a_name_in_the_same_list() {
    for source in [
        "read(value, value): return value\n",
        "Record has\n    read(self ref, value, value): return value\n",
        "main:\n    read(value, value): return value\n    return 0\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .any(|diagnostic| diagnostic.code.slug() == "parse-duplicate-parameter"),
            "{source}: {:?}",
            parsed.symptoms,
        );
    }
}

#[test]
fn separate_parameter_lists_can_reuse_names() {
    let parsed = "first(value): return value\nsecond(value): return value\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
}
