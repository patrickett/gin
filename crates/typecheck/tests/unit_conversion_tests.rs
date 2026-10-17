mod support;

use support::flaw_codes_of;

fn unit_types() -> &'static str {
    "Byte is in 0 to 255\nCodePoint is in 0 to 1114111\n#phantom(unit)\nCount(unit) is in 0 to 127\n#phantom(unit)\nOffset(unit) is in -127 to 127\n"
}

#[test]
fn count_to_same_unit_offset_is_lossless() {
    let source = format!(
        "{}convert(value Count(Byte)) Offset(Byte): return value as Offset(Byte)\n",
        unit_types()
    );
    let codes = flaw_codes_of(&source);
    assert!(
        !codes
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing")),
        "{codes:?}"
    );
}

#[test]
fn unrestricted_offset_to_count_requires_nonnegative_proof() {
    let source = format!(
        "{}convert(value Offset(Byte)) Count(Byte): return value as Count(Byte)\n",
        unit_types()
    );
    let codes = flaw_codes_of(&source);
    assert!(
        codes
            .iter()
            .any(|code| code == "type-integer-narrowing-failed"),
        "{codes:?}"
    );
}

#[test]
fn cross_unit_conversion_requires_explicit_cast() {
    let source = format!(
        "{}convert(value Count(Byte)) Count(CodePoint): return value\n",
        unit_types()
    );
    let codes = flaw_codes_of(&source);
    assert!(
        codes.iter().any(|code| code == "type-return-mismatch"),
        "{codes:?}"
    );
}

#[test]
fn self_constructor_satisfies_an_applied_generic_return_type() {
    let codes = flaw_codes_of(
        "Range(x) has\n    start x\n    end x\n    new(start x, end x) Range(x): return Self(start, end)\n",
    );
    assert!(
        !codes.iter().any(|code| code == "type-return-mismatch"),
        "{codes:?}"
    );
}
