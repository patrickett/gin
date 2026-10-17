use diagnostic::Category;
use parser::query::SourceParseExt;

#[test]
fn empty_bind_parameters_suggest_bare_name() {
    let output = "good() Int: return 7\n".parse_source_full();
    let warnings: Vec<_> = output
        .symptoms
        .iter()
        .filter(|d| d.code.slug() == "parse-empty-parens")
        .collect();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].category, Category::Warn);
    assert_eq!(
        warnings[0].help.as_deref(),
        Some("use good instead of good()")
    );
    assert!(!output.symptoms.iter().any(|d| d.category == Category::Flaw));
}

#[test]
fn bare_bind_and_nonempty_parameters_do_not_warn() {
    for source in ["good Int: 7\n", "good(value Int) Int: return value\n"] {
        let output = source.parse_source_full();
        assert!(
            !output
                .symptoms
                .iter()
                .any(|d| d.code.slug() == "parse-empty-parens")
        );
    }
}

#[test]
fn nested_bind_and_call_both_warn() {
    let output = "outer Int:\n    inner() Int: return 7\n    return inner()\n".parse_source_full();
    assert_eq!(
        output
            .symptoms
            .iter()
            .filter(|d| d.code.slug() == "parse-empty-parens")
            .count(),
        2
    );
}

#[test]
fn associated_callable_parameters_are_required() {
    {
        let source = "Encoder has\n    encode() Byte: return 0\n";
        let output = source.parse_source_full();
        assert!(
            !output
                .symptoms
                .iter()
                .any(|diagnostic| diagnostic.code.slug() == "parse-empty-parens"),
            "{:?}",
            output.symptoms
        );
    }
}
