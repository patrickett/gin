mod support;

use parser::query::SourceParseExt;
use support::{check, flaw_codes};

fn flaw_messages(typed: &typecheck::TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| format!("{}: {}", flaw.code.slug(), flaw.message))
        .collect()
}

#[test]
fn removed_condition_attribute_does_not_create_a_condition_contract() {
    let typed = check("#condition\nTruth is True or False\n");
    let codes = flaw_codes(&typed);
    assert!(
        !codes.iter().any(|code| code.contains("condition-contract")),
        "{}",
        format!("{:?}", flaw_messages(&typed))
    );
}

#[test]
fn removed_auto_attribute_is_rejected() {
    let parse_output = "#auto\nBad is in 0 to 255".parse_source_full();
    let codes: Vec<_> = parse_output
        .symptoms
        .iter()
        .map(|diag| diag.code.to_string())
        .collect();

    assert!(
        codes.contains(&"parse-removed-auto-attribute".to_string()),
        "{codes:?}"
    );
}

#[test]
fn integer_values_require_an_explicit_condition_pattern() {
    let parsed = "main: when 1 then 1 else 0\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = typecheck::transform::transform_file(parsed.ast, typecheck::FileId(0));
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-condition-requires-proposition")
    );
}
