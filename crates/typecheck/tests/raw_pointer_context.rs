mod support;

use support::flaw_codes_of;

#[test]
fn nominal_raw_pointer_context_is_propagated_to_address_taking() {
    let codes = flaw_codes_of(&format!(
        "{}PointerToWord is @Word\nPointerToWord has\n    finish(self):\n        return\nentry(value Word) Word:\n    address PointerToWord: @value\n    return 0\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(codes.is_empty(), "unexpected diagnostics: {codes:?}");
}

#[test]
fn address_taking_without_pointer_context_remains_rejected() {
    let codes = flaw_codes_of(&format!(
        "{}make(value Word): return @value\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert_eq!(
        codes,
        vec!["type-raw-pointer-requires-expected-type".to_string()]
    );
}
