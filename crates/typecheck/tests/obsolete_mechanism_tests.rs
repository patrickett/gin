use parser::cursor::TokenCursor;
use parser::query::SourceParseExt;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn check(source: &str) -> typecheck::TypedFileAst {
    let ast = TokenCursor::parse_source(source);
    transform(&ast, FileId(0), &TransformCtx::new())
}

fn flaw_codes(typed: &typecheck::TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.slug().to_string())
        .collect()
}

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
    let parse_output = "#auto\nBad is in 0...255".parse_source_full();
    let codes: Vec<_> = parse_output
        .symptoms
        .iter()
        .map(|diag| diag.code.slug().to_string())
        .collect();

    assert!(
        codes.contains(&"parse-removed-auto-attribute".to_string()),
        "{codes:?}"
    );
}

#[test]
fn if_without_condition_pattern_is_rejected_by_parser() {
    let parsed = "main:\n    if 1\n        1\n    return\n    return 0\n".parse_source_full();
    let codes = parsed
        .symptoms
        .iter()
        .map(|diagnostic| diagnostic.code.slug().to_string())
        .collect::<Vec<_>>();
    assert!(
        codes.contains(&"parse-expected-condition-pattern".to_string()),
        "expected condition pattern requirement, got: {:?}",
        codes
    );
}
