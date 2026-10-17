mod common;

use common::assert_roundtrip;
#[test]
fn emission_expression_round_trips() {
    let source = "run(value Word) Word: return #emit(code, Inputs(value: value))\n";
    let formatted = ginfmt::format(source);
    let parsed = parser::parse_from_str(&formatted);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{formatted}\n{:?}",
        parsed.parse_warnings
    );
    assert!(formatted.contains("#emit(code, Inputs("), "{formatted}");
    assert_roundtrip(&formatted);
}

#[test]
fn emission_signature_types_round_trip() {
    for source in [
        "signature Signature(Inputs, ()): Signature(version: 1)\n",
        "signature Signature([pointer ref Word, length Len], [value Word]): Signature(version: 1)\n",
    ] {
        let formatted = ginfmt::format(source);
        let parsed = parser::parse_from_str(&formatted);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{formatted}\n{:?}",
            parsed.parse_warnings
        );
        assert_roundtrip(&formatted);
    }
}
