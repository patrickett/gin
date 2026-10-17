use parser::query::SourceParseExt;

#[test]
fn automatic_destruction_queries_are_deferred() {
    for query in [
        "#auto_destroy",
        "#auto_destroy(last_use)",
        "#auto_destroy(end_of_scope)",
    ] {
        let parsed = format!("Resource has\n    {query}\n    release(self):\n        return\n")
            .parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .any(|flaw| { flaw.code.slug() == "parse-deferred-auto-destroy" }),
            "{:?}",
            parsed.symptoms
        );
    }
}

#[test]
fn automatic_destruction_cannot_be_attached_to_a_type() {
    let parsed = "#auto_destroy(end_of_scope)\nResource has\n    release(self):\n        return\n"
        .parse_source_full();
    assert!(
        parsed
            .symptoms
            .iter()
            .any(|flaw| { flaw.code.slug() == "parse-deferred-auto-destroy" }),
        "{:?}",
        parsed.symptoms
    );
}
