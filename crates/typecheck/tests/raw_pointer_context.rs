use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn flaws(source: &str) -> Vec<String> {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
    .all_flaws()
    .into_iter()
    .map(|(_, flaw)| flaw.code.slug().to_string())
    .collect()
}

#[test]
fn nominal_raw_pointer_context_is_propagated_to_address_taking() {
    let codes = flaws(
        r#"Word is in 0 to 255
PointerToWord is @Word
entry(value Word) Word:
    address PointerToWord: @value
    return 0
"#,
    );
    assert!(codes.is_empty(), "unexpected diagnostics: {codes:?}");
}

#[test]
fn address_taking_without_pointer_context_remains_rejected() {
    let codes = flaws("Word is in 0 to 255\nmake(value Word): @value\n");
    assert_eq!(
        codes,
        vec!["type-raw-pointer-requires-expected-type".to_string()]
    );
}
