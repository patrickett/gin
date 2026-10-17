use parser::query::SourceParseExt;
use typecheck::{FileId, transform::transform_file};

const DECLARATIONS: &str = include_str!("../../test-fixtures/fixtures/callable_small_proof.gin");

#[test]
fn callable_proof_applies_only_to_the_argument_version_it_checked() {
    for (condition, valid) in [
        ("small(value) is True", true),
        ("value < 16", true),
        ("grow(value) is True and value < 16", true),
        ("value < 16 and grow(value) is True", false),
        ("grow(value) is True and small(value) is True", true),
        ("small(value) is True and grow(value) is True", false),
    ] {
        let source = format!(
            "{DECLARATIONS}narrow(value mut is in 0 to 255) Nibble:\n    when {condition} then\n        number is in 0 to 255: value\n        return number as Nibble\n    return 0\n"
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{condition}: {:?}",
            parsed.symptoms
        );
        let typed = transform_file(parsed.ast, FileId(0));
        let flaws = typed.all_flaws();
        if valid {
            assert!(flaws.is_empty(), "{condition}: {flaws:?}");
        } else {
            assert!(
                flaws.iter().any(|(_, flaw)| matches!(
                    flaw.code.slug().as_str(),
                    "type-integer-narrowing-failed" | "type-integer-narrowing-unproven"
                )),
                "stale proof accepted: {flaws:?}"
            );
        }
    }
}

#[test]
fn range_proof_applies_only_to_the_argument_version_it_checked() {
    for (condition, valid) in [
        ("value is in 0 to 15", true),
        ("grow(value) is True and value is in 0 to 15", true),
        ("value is in 0 to 15 and grow(value) is True", false),
    ] {
        let source = format!(
            "{DECLARATIONS}narrow(value mut is in 0 to 255) Nibble:\n    when {condition} then\n        number is in 0 to 255: value\n        return number as Nibble\n    return 0\n"
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{condition}: {:?}",
            parsed.symptoms
        );
        let typed = transform_file(parsed.ast, FileId(0));
        let flaws = typed.all_flaws();
        if valid {
            assert!(flaws.is_empty(), "{condition}: {flaws:?}");
        } else {
            assert!(
                flaws.iter().any(|(_, flaw)| matches!(
                    flaw.code.slug().as_str(),
                    "type-integer-narrowing-failed" | "type-integer-narrowing-unproven"
                )),
                "stale range accepted: {flaws:?}"
            );
        }
    }
}

#[test]
fn a_numeric_comparison_does_not_refine_a_value_changed_by_its_right_operand() {
    let source = format!(
        "{DECLARATIONS}reset(value mut is in 0 to 255) is in 0 to 255:\n    value :: 255\n    return 16\nnarrow(value mut is in 0 to 255) Nibble:\n    when value < reset(value) then\n        number is in 0 to 255: value\n        return number as Nibble\n    return 0\n"
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast, FileId(0));
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| matches!(
            flaw.code.slug().as_str(),
            "type-integer-narrowing-failed" | "type-integer-narrowing-unproven"
        )),
        "{:?}",
        typed.all_flaws()
    );
}
