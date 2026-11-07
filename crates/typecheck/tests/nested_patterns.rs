//! `Flaw(OutOfMemory)`: a case arm names a variant of the payload union too.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_file_diagnostic, check_module};

const TYPES: &str = "\
#unrestricted
AllocError is OutOfMemory or Fragmented

#unrestricted
Code is in 0 to 9

Outcome is Value(Code) or Flaw(AllocError)

";

fn run(tail: &str) -> Result<(), SourceCheckError> {
    let text = format!("{TYPES}{tail}");
    let file = parser::parse_strict(&text).unwrap_or_else(|d| panic!("{text}\n{d:?}"));
    check_file_diagnostic(&file, Limits::default()).map(|_| ()).map_err(|failure| failure.error)
}

#[test]
fn an_exhaustive_nested_match_is_accepted() {
    let full = "f(r Outcome) in 0 to 3: return when r is\n    Value(_) then 0\n    Flaw(OutOfMemory) then 1\n    Flaw(Fragmented) then 2\n";
    assert_eq!(run(full), Ok(()));
    // Arms of one variant need not be adjacent, and a binder of the outer variant's other fields works.
    let apart = "f(r Outcome) in 0 to 3: return when r is\n    Flaw(OutOfMemory) then 1\n    Value(_) then 0\n    Flaw(Fragmented) then 2\n";
    assert_eq!(run(apart), Ok(()));
}

#[test]
fn a_missing_inner_variant_is_refused() {
    let missing = "f(r Outcome) in 0 to 3: return when r is\n    Value(_) then 0\n    Flaw(OutOfMemory) then 1\n";
    assert_eq!(run(missing), Err(SourceCheckError::Type));
}

#[test]
fn a_duplicate_inner_variant_is_refused() {
    let twice = "f(r Outcome) in 0 to 3: return when r is\n    Value(_) then 0\n    Flaw(OutOfMemory) then 1\n    Flaw(OutOfMemory) then 2\n    Flaw(Fragmented) then 2\n";
    assert_eq!(run(twice), Err(SourceCheckError::Duplicate));
}

#[test]
fn a_wrong_inner_variant_name_is_refused() {
    let wrong = "f(r Outcome) in 0 to 3: return when r is\n    Value(_) then 0\n    Flaw(OutOfMemory) then 1\n    Flaw(Nonexistent) then 2\n";
    assert!(run(wrong).is_err());
    // Nesting inside a variant that holds no union.
    let plain = "f(r Outcome) in 0 to 3: return when r is\n    Value(OutOfMemory) then 0\n    Flaw(_) then 1\n";
    assert_eq!(run(plain), Err(SourceCheckError::Type));
}

#[test]
fn a_whole_payload_arm_beside_nested_arms_is_refused() {
    let both = "f(r Outcome) in 0 to 3: return when r is\n    Value(_) then 0\n    Flaw(OutOfMemory) then 1\n    Flaw(other) then 2\n";
    assert!(run(both).is_err());
}

#[test]
fn a_guarded_nested_arm_needs_an_unguarded_one() {
    // A guarded nested arm does not cover its inner variant.
    let guarded = "f(r Outcome) in 0 to 3: return when r is\n    Value(_) then 0\n    Flaw(OutOfMemory) and 1 = 1 then 1\n    Flaw(Fragmented) then 2\n";
    assert_eq!(run(guarded), Err(SourceCheckError::Type));
    let fallback = "f(r Outcome, n in 0 to 9) in 0 to 3: return when r is\n    Value(_) then 0\n    Flaw(OutOfMemory) and n > 4 then 1\n    Flaw(OutOfMemory) then 3\n    Flaw(Fragmented) then 2\n";
    assert_eq!(run(fallback), Ok(()));
    // The guard is checked: a false claim in the guarded arm's body is still found.
    let bad = fallback.replace("and n > 4 then 1", "and n > 4 then missing");
    assert!(run(&bad).is_err());
}

#[test]
fn the_nested_arms_see_the_inner_payload() {
    let source = "\
Token has n in 0 to 3

Held is Some(Token) or None

Wrapped is Inside(Held) or Outside

take(w Wrapped) in 0 to 3: return when w is
    Outside then 0
    Inside(Some(token)) then
        Token(n) := token
        return n
    Inside(None) then 1
";
    assert_eq!(run(source), Ok(()));
    // A linear payload bound by a nested pattern must still be consumed.
    let leaked = source.replace("        Token(n) := token\n        return n\n", "        return 2\n");
    assert_eq!(run(&leaked), Err(SourceCheckError::Unconsumed));
}

#[test]
fn a_nested_match_works_across_modules() {
    let library_text = format!("{TYPES}classify(code in 0 to 9) Outcome: return when code < 5 then Value(code as Code) else Flaw(OutOfMemory)\n");
    let library_file = parser::parse_strict(&library_text).unwrap();
    let library: CheckedSourceModule = check_module("lib.outcome", &library_file, &[], Limits::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let client = "use lib.outcome Outcome, classify\n\nrank(n in 0 to 9) in 0 to 3: return when classify(n) is\n    Value(_) then 0\n    Flaw(OutOfMemory) then 1\n    Flaw(Fragmented) then 2\n";
    let file = parser::parse_strict(client).unwrap();
    check_module("app.client", &file, &[&library], Limits::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let missing = client.replace("    Flaw(Fragmented) then 2\n", "");
    let file = parser::parse_strict(&missing).unwrap();
    assert_eq!(check_module("app.client", &file, &[&library], Limits::default()).unwrap_err(), SourceCheckError::Type);
}

#[test]
fn a_subject_fixed_by_construction_still_checks_every_arm() {
    let ok = "f() in 0 to 3:\n    r Outcome: Flaw(Fragmented)\n    return when r is\n        Value(_) then 0\n        Flaw(OutOfMemory) then 1\n        Flaw(Fragmented) then 2\n";
    assert_eq!(run(ok), Ok(()));
    // An arm no world selects is still checked: its body is wrong.
    let bad = "f() in 0 to 3:\n    r Outcome: Flaw(Fragmented)\n    return when r is\n        Value(_) then 0\n        Flaw(OutOfMemory) then missing\n        Flaw(Fragmented) then 2\n";
    assert!(run(bad).is_err());
}

#[test]
fn a_generic_union_nests_too() {
    let source = "\
#unrestricted
Step(element) is Yield(element) or Done

#unrestricted
Pair(element) is First(Step(element)) or Second

g(p Pair(Code)) in 0 to 2: return when p is
    Second then 0
    First(Yield(_)) then 1
    First(Done) then 2
";
    assert_eq!(run(source), Ok(()));
}

#[test]
fn a_linear_payload_bound_by_a_nested_pattern_is_consumed_exactly_once() {
    let twice = "\
Token has n in 0 to 3

Held is Some(Token) or None

Wrapped is Inside(Held) or Outside

take(w Wrapped) in 0 to 3: return when w is
    Outside then 0
    Inside(Some(token)) then
        Token(n) := token
        Token(m) := token
        return n
    Inside(None) then 1
";
    let error = run(twice).unwrap_err();
    assert_ne!(error, SourceCheckError::Unconsumed, "{error:?}");
}

#[test]
fn a_labeled_nested_pattern_takes_the_field_it_names() {
    let types = "\
#unrestricted
AllocError is OutOfMemory or Fragmented

#unrestricted
Code is in 0 to 9

Outcome is Value(Code) or Flaw(code Code, error AllocError)

";
    let check = |tail: &str| {
        let text = format!("{types}{tail}");
        let file = parser::parse_strict(&text).map_err(|_| SourceCheckError::Malformed)?;
        check_file_diagnostic(&file, Limits::default()).map(|_| ()).map_err(|failure| failure.error)
    };
    let arms = |flaw: &str, frag: &str| format!("f(r Outcome) in 0 to 9: return when r is\n    Value(_) then 0\n    {flaw} then 1\n    {frag} then c\n");
    assert_eq!(check(&arms("Flaw(_: code, OutOfMemory: error)", "Flaw(c: code, Fragmented: error)")), Ok(()));
    // Either order of the labels.
    assert_eq!(check(&arms("Flaw(OutOfMemory: error, _: code)", "Flaw(Fragmented: error, c: code)")), Ok(()));
    // Negative controls: an unknown field label, a field named twice, a missing inner variant.
    assert_eq!(check(&arms("Flaw(_: code, OutOfMemory: missing)", "Flaw(c: code, Fragmented: error)")), Err(SourceCheckError::Type));
    assert_eq!(check(&arms("Flaw(_: error, OutOfMemory: error)", "Flaw(c: code, Fragmented: error)")), Err(SourceCheckError::Duplicate));
    assert_eq!(check("f(r Outcome) in 0 to 9: return when r is\n    Value(_) then 0\n    Flaw(_: code, OutOfMemory: error) then 1\n"), Err(SourceCheckError::Type));
    // The field-first spelling is not the label direction and stays a parse error.
    assert_eq!(check(&arms("Flaw(code: _, error: OutOfMemory)", "Flaw(c: code, Fragmented: error)")), Err(SourceCheckError::Malformed));
}

const SLOTS: &str = "\
#unrestricted
Count is in 0 to 9

#unrestricted
Slot is Empty or Full(Count)

#unrestricted
Step is Done(tag Count, value Slot) or Pending

";

fn slots(tail: &str) -> Result<(), SourceCheckError> {
    let text = format!("{SLOTS}{tail}");
    let file = parser::parse_strict(&text).map_err(|_| SourceCheckError::Malformed)?;
    check_file_diagnostic(&file, Limits::default()).map(|_| ()).map_err(|failure| failure.error)
}

#[test]
fn field_first_is_a_construction_and_pattern_first_is_a_pattern() {
    // Construction: `value: Full(v)` sets the field `value`, `v` already in scope.
    assert_eq!(slots("make(v Count) Step: return Done(tag: v, value: Full(v))\n"), Ok(()));
    assert_eq!(slots("make(v Count) Step: return Step.Done(tag: v, value: Slot.Full(v))\n"), Ok(()));
    // In a pattern the pattern comes first.
    let arms = "read(s Step) in 0 to 9: return when s is\n    Pending then 0\n    Done(Empty: value, _: tag) then 1\n    Done(Full(v): value, _: tag) then v\n";
    assert_eq!(slots(arms), Ok(()));
    let text = format!("{SLOTS}{}", arms.replace("Done(Full(v): value, _: tag)", "Done(value: Full(v), _: tag)"));
    let message = format!("{:?}", parser::parse_strict(&text).unwrap_err());
    assert!(message.contains("in a pattern the pattern comes first"), "{message}");
}

#[test]
fn a_typed_parameter_default_is_not_supported_yet_even_for_a_nested_construction() {
    // Records have no field defaults; a parameter default with a type is refused by the parser,
    // whatever its value (`3`, `Pending`, `Done(tag: .., value: Full(..))`).
    for default in ["s Step: Pending", "s Step: Done(tag: 1 as Count, value: Full(2 as Count))"] {
        let text = format!("{SLOTS}make({default}) in 0 to 9: return 0\n");
        let message = format!("{:?}", parser::parse_strict(&text).unwrap_err());
        assert!(message.contains("typed parameter defaults are not supported"), "{message}");
    }
}

const DEEP: &str = "\
#unrestricted
Code is in 0 to 9

#unrestricted
Inner is Low or High(Code)

#unrestricted
Slot is Empty or Full(Inner)

#unrestricted
Step is Done(Slot) or Pending

";

fn deep(tail: &str) -> Result<(), SourceCheckError> {
    let text = format!("{DEEP}{tail}");
    let file = parser::parse_strict(&text).unwrap_or_else(|d| panic!("{text}\n{d:?}"));
    check_file_diagnostic(&file, Limits::default()).map(|_| ()).map_err(|failure| failure.error)
}

const ARMS: &str = "f(s Step) in 0 to 9: return when s is\n    Pending then 0\n    Done(Empty) then 1\n    Done(Full(Low)) then 2\n    Done(Full(High(n))) then n\n";

#[test]
fn a_pattern_two_levels_deep_is_covered_as_a_tree() {
    assert_eq!(deep(ARMS), Ok(()));
    // Arms in another order, and one level stopping where the other goes deeper.
    let mixed = "f(s Step) in 0 to 9: return when s is\n    Done(Full(High(n))) then n\n    Pending then 0\n    Done(Full(Low)) then 2\n    Done(Empty) then 1\n";
    assert_eq!(deep(mixed), Ok(()));
}

#[test]
fn two_levels_deep_negative_controls() {
    // A missing leaf.
    assert_eq!(deep(&ARMS.replace("    Done(Full(Low)) then 2\n", "")), Err(SourceCheckError::Type));
    // The same leaf twice.
    assert_eq!(deep(&ARMS.replace("    Done(Full(Low)) then 2\n", "    Done(Full(Low)) then 2\n    Done(Full(Low)) then 3\n")), Err(SourceCheckError::Duplicate));
    // A whole payload beside a pattern inside it.
    assert_eq!(deep(&ARMS.replace("    Done(Full(Low)) then 2\n", "    Done(Full(Low)) then 2\n    Done(Full(x)) then 3\n")), Err(SourceCheckError::Duplicate));
    // A wrong name at the second level.
    assert!(deep(&ARMS.replace("Done(Full(Low))", "Done(Full(Nowhere))")).is_err());
    // A pattern inside a variant that holds no union.
    assert_eq!(deep(&ARMS.replace("Done(Full(High(n))) then n", "Done(Full(High(Low))) then 3")), Err(SourceCheckError::Type));
}

#[test]
fn a_linear_payload_two_levels_deep_is_consumed() {
    let source = "\
Token has n in 0 to 3

Held is Some(Token) or None

Wrapped is Inside(Held) or Outside

Outer is Boxed(Wrapped) or Bare

take(o Outer) in 0 to 3: return when o is
    Bare then 0
    Boxed(Outside) then 1
    Boxed(Inside(None)) then 2
    Boxed(Inside(Some(token))) then
        Token(n) := token
        return n
";
    assert_eq!(run(source), Ok(()));
    let leaked = source.replace("        Token(n) := token\n        return n\n", "        return 3\n");
    assert_eq!(run(&leaked), Err(SourceCheckError::Unconsumed));
}
