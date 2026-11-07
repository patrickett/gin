//! `Flaw(OutOfMemory)`: a pattern in place of a binder, in a case arm.
#[test]
fn a_case_arm_may_nest_a_pattern() {
    let source = "f(r Result) in 0 to 2: return when r is\n    Flaw(OutOfMemory) then 0\n    Flaw(other) then 1\n    Value(n) then 2\n";
    let file = parser::parse_strict(source).unwrap_or_else(|d| panic!("{d:?}"));
    let debug = format!("{file:?}");
    assert!(debug.contains("nested: [Some("), "{debug}");
}

#[test]
fn deeper_nesting_parses() {
    parser::parse_strict("f(r Result) in 0 to 2: return when r is\n    Done(Full(Inner(v))) then 0\n    Done(Full(_)) then 1\n    Empty then 2\n").unwrap_or_else(|d| panic!("{d:?}"));
}

#[test]
fn only_a_case_arm_nests() {
    // Negative controls: a destructure, a one-line test and a guard condition stay flat.
    assert!(parser::parse_strict("f(r Result):\n    Flaw(OutOfMemory) := r\n    return\n").is_err());
    assert!(parser::parse_strict("f(r Result) in 0 to 1: return when r is Flaw(OutOfMemory) then 0 else 1\n").is_err());
}

// ---- audit additions (2026-11-05) ----

fn message(source: &str) -> String {
    let error = parser::parse_strict(source).expect_err(source);
    error.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
}

#[test]
fn the_labeled_form_puts_the_pattern_first() {
    let ok = "f(s Step) in 0 to 2: return when s is\n    Done(Full(v): value, _: tag) and v > 1 then 1\n    Done(Empty: value, _: tag) then 0\n";
    let file = parser::parse_strict(ok).unwrap_or_else(|d| panic!("{d:?}"));
    assert!(format!("{file:?}").contains("nested: [Some("));
    // Negative control: the construction order (`field: Pattern`) is rejected in a pattern with
    // a message that says which way round it goes, not a generic one.
    let said = message(&ok.replace("Full(v): value", "value: Full(v)"));
    assert!(said.contains("in a pattern the pattern comes first"), "{said}");
}

#[test]
fn the_same_text_is_a_construction_outside_a_pattern() {
    // `value: Full(v)` is fine as an argument: the parser must not have lost it.
    parser::parse_strict("make(v Count) Step: return Done(tag: v, value: Full(v))\n").unwrap_or_else(|d| panic!("{d:?}"));
}

#[test]
fn flat_only_positions_say_that_nested_patterns_are_unsupported() {
    for source in [
        "f(r Result):\n    Flaw(OutOfMemory) := r\n    return\n",
        "f(r Result) in 0 to 1: return when r is Flaw(OutOfMemory) then 0 else 1\n",
    ] {
        let said = message(source);
        assert!(said.contains("nested patterns are unsupported"), "{source}: {said}");
    }
}
