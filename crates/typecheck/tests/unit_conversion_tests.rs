use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn flaw_codes(source: &str) -> Vec<String> {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
    .all_flaws()
    .into_iter()
    .map(|(_, diagnostic)| diagnostic.code.slug().to_string())
    .collect()
}

fn unit_types() -> &'static str {
    "Byte is in 0...255\nCodePoint is in 0...1114111\n#phantom(unit)\nCount(unit) is in 0...127\n#phantom(unit)\nOffset(unit) is in -127...127\n"
}

#[test]
fn count_to_same_unit_offset_is_lossless() {
    let source = format!(
        "{}convert(value Count(Byte)) Offset(Byte): value as Offset(Byte)\n",
        unit_types()
    );
    let codes = flaw_codes(&source);
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
        "{}convert(value Offset(Byte)) Count(Byte): value as Count(Byte)\n",
        unit_types()
    );
    let codes = flaw_codes(&source);
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
        "{}convert(value Count(Byte)) Count(CodePoint): value\n",
        unit_types()
    );
    let codes = flaw_codes(&source);
    assert!(
        codes.iter().any(|code| code == "type-return-mismatch"),
        "{codes:?}"
    );
}

#[test]
fn self_constructor_satisfies_an_applied_generic_return_type() {
    let codes = flaw_codes(
        "Range(x) has\n    start x\n    end x\n    new(start x, end x) Range(x): Self(start, end)\n",
    );
    assert!(
        !codes.iter().any(|code| code == "type-return-mismatch"),
        "{codes:?}"
    );
}
