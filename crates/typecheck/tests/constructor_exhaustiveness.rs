use parser::query::SourceParseExt;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn flaw_codes(arms: &str) -> Vec<String> {
    let source = format!(
        "Byte is in 0 to 255\nByte.finish(self):\n    return\nOptional is Some(value Byte) or None\nOptional.finish(self): return when self is Some(value) then value.finish else idle\nidle:\n    return\nread(value ref Optional) Byte: return when value is\n{arms}\n"
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    transform(&parsed.ast, FileId(0), &TransformCtx::new())
        .all_flaws()
        .iter()
        .map(|(_, flaw)| flaw.code.to_string())
        .collect()
}

#[test]
fn constructor_wildcard_leaves_other_constructor_arms_reachable() {
    let flaws = flaw_codes("    Some(_) then 1\n    None then 0");
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn constructor_wildcard_still_needs_the_other_variant() {
    let flaws = flaw_codes("    Some(_) then 1");
    assert!(
        flaws.iter().any(|code| code == "type-missing-else-arm"),
        "{flaws:?}"
    );
}

#[test]
fn constructor_wildcard_allows_else_for_other_variants() {
    let flaws = flaw_codes("    Some(_) then 1\n    else 0");
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn constrained_constructor_payload_still_needs_else() {
    let flaws = flaw_codes("    Some(0) then 1\n    None then 0");
    assert!(
        flaws.iter().any(|code| code == "type-missing-else-arm"),
        "{flaws:?}"
    );
}

fn pair_flaw_codes(arms: &str) -> Vec<String> {
    let source = format!(
        "Byte is in 0 to 255\nByte.finish(self):\n    return\nBit is in 0 to 1\nBit.finish(self):\n    return\nChoice is Pair(first Bit, second Bit) or None\nChoice.finish(self): return when self is Pair(first, second) then finish_bits(first, second) else idle\nfinish_bits(first Bit, second Bit):\n    first.finish\n    second.finish\n    return\nidle:\n    return\nread(value ref Choice) Byte: return when value is\n{arms}\n"
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    transform(&parsed.ast, FileId(0), &TransformCtx::new())
        .all_flaws()
        .iter()
        .map(|(_, flaw)| flaw.code.to_string())
        .collect()
}

#[test]
fn constructor_payload_cases_collectively_cover_both_fields() {
    let flaws = pair_flaw_codes(
        "    Pair(0, 0) then 0\n    Pair(0, 1) then 1\n    Pair(1, 0) then 2\n    Pair(1, 1) then 3\n    None then 4",
    );
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn independently_covered_fields_do_not_prove_all_combinations() {
    let flaws = pair_flaw_codes("    Pair(0, 0) then 0\n    Pair(1, 1) then 1\n    None then 2");
    assert!(
        flaws.iter().any(|code| code == "type-missing-else-arm"),
        "{flaws:?}"
    );
}

#[test]
fn payload_rows_can_combine_captures_and_literals() {
    let flaws =
        pair_flaw_codes("    Pair(0, second) then 0\n    Pair(1, second) then 1\n    None then 2");
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn repeated_payload_literals_in_different_positions_remain_reachable() {
    let source = "Byte is in 0 to 255\nByte.finish(self):\n    return\nBit is in 0 to 1\nBit.finish(self):\n    return\nChoice is Many(first Bit, second Bit, third Bit) or None\nChoice.finish(self): return when self is Many(first, second, third) then finish_bits(first, second, third) else idle\nfinish_bits(first Bit, second Bit, third Bit):\n    first.finish\n    second.finish\n    third.finish\n    return\nidle:\n    return\nread(value ref Choice) Byte: return when value is\n    Many(0, 0, 1) then 0\n    Many(0, 1, 0) then 1\n    else 2\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform(&parsed.ast, FileId(0), &TransformCtx::new());
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
