//! Real source lifecycle/failure witnesses using only public, body-checked capsules.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

const SLOTS: &str = include_str!("../examples/runtime-slots.gin");
const BUFFER: &str = include_str!("../examples/indexed-buffer.gin");
const LIFECYCLE: &str = include_str!("../examples/indexed-buffer-lifecycle.gin");
const BUILD: &str = include_str!("../examples/indexed-buffer-build.gin");
const CLIENT: &str = include_str!("../examples/indexed-buffer-lifecycle-client.gin");
const BUILD_CLIENT: &str = include_str!("../examples/indexed-buffer-build-client.gin");

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict lifecycle source syntax");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    result
}

fn buffer() -> CheckedSourceModule {
    let slots = check("example.slots", SLOTS, &[]).unwrap();
    check("example.buffers", BUFFER, &[&slots]).unwrap()
}

fn lifecycle(buffer: &CheckedSourceModule) -> CheckedSourceModule {
    check("example.lifecycle", LIFECYCLE, &[buffer]).unwrap()
}

fn mutate(text: &str, before: &str, after: &str) -> String {
    assert_eq!(text.matches(before).count(), 1, "unique mutation target");
    let mutant = text.replace(before, after);
    assert_ne!(mutant, text);
    mutant
}

#[test]
fn drain_ends_all_four_occupancy_patterns_and_preserves_scalar_identity() {
    let buffer = buffer();
    let lifecycle = lifecycle(&buffer);
    let observers = "
empty_zero in 0 to 16 and = 0:
    Ended(zero, one) := empty
    return zero
empty_one in 0 to 16 and = 0:
    Ended(zero, one) := empty
    return one
zero_present(number in 1 to 16) in 0 to 16 and = number:
    Ended(zero, one) := zero_only(number)
    return zero
zero_other(number in 1 to 16) in 0 to 16 and = 0:
    Ended(zero, one) := zero_only(number)
    return one
one_other(number in 1 to 16) in 0 to 16 and = 0:
    Ended(zero, one) := one_only(number)
    return zero
one_present(number in 1 to 16) in 0 to 16 and = number:
    Ended(zero, one) := one_only(number)
    return one
both_zero(first in 1 to 16, second in 1 to 16) in 0 to 16 and = first:
    Ended(zero, one) := both(first, second)
    return zero
both_one(first in 1 to 16, second in 1 to 16) in 0 to 16 and = second:
    Ended(zero, one) := both(first, second)
    return one
";
    let source = format!("{CLIENT}{observers}");
    check("example.lifecycle_client", &source, &[&buffer, &lifecycle]).unwrap();
}

#[test]
fn missing_payload_cleanup_or_empty_buffer_close_rejects() {
    let buffer = buffer();
    for (before, after) in [
        ("number: end_token(value)", "number: 0"),
        ("    Closed := close_buffer(Token, empty)\n", ""),
    ] {
        let mutant = mutate(LIFECYCLE, before, after);
        assert_eq!(
            check("example.lifecycle", &mutant, &[&buffer]).unwrap_err(),
            SourceCheckError::Unconsumed
        );
    }
}

#[test]
fn duplicate_ending_and_reusing_the_original_buffer_reject() {
    let buffer = buffer();
    let duplicate = format!(
        "{LIFECYCLE}
duplicate_end(value Token) in 1 to 16:
    number: end_token(value)
    again: end_token(value)
    return number
"
    );
    assert_eq!(
        check("example.lifecycle", &duplicate, &[&buffer]).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
    let reused = mutate(
        LIFECYCLE,
        "buffer: rest, number: end_token(value)",
        "buffer: buffer, number: end_token(value)",
    );
    assert_eq!(
        check("example.lifecycle", &reused, &[&buffer]).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
}

#[test]
fn partial_build_failure_and_success_have_explicit_cleanup() {
    let buffer = buffer();
    let lifecycle = lifecycle(&buffer);
    let build = check("example.build", BUILD, &[&buffer, &lifecycle]).unwrap();
    let observers = "
failed_first(first in 1 to 16, second in 1 to 16) in 0 to 16 and = first:
    Aborted(ended, proposal_number) := failed(first, second)
    Ended(zero, one) := ended
    return zero
failed_second_empty(first in 1 to 16, second in 1 to 16) in 0 to 16 and = 0:
    Aborted(ended, proposal_number) := failed(first, second)
    Ended(zero, one) := ended
    return one
failed_proposal(first in 1 to 16, second in 1 to 16) in 1 to 16 and = second:
    Aborted(ended, proposal_number) := failed(first, second)
    Ended(zero, one) := ended
    return proposal_number
succeeded_first(first in 1 to 16, second in 1 to 16) in 0 to 16 and = first:
    Ended(zero, one) := succeeded(first, second)
    return zero
succeeded_second(first in 1 to 16, second in 1 to 16) in 0 to 16 and = second:
    Ended(zero, one) := succeeded(first, second)
    return one
";
    let source = format!("{BUILD_CLIENT}{observers}");
    check(
        "example.build_client",
        &source,
        &[&buffer, &lifecycle, &build],
    )
    .unwrap();
}

#[test]
fn failure_cannot_lose_either_the_partial_buffer_or_the_proposal() {
    let buffer = buffer();
    let lifecycle = lifecycle(&buffer);
    for (before, after) in [
        (
            "Failed(buffer: partial, proposal: second)",
            "Failed(buffer: empty_buffer(Token), proposal: second)",
        ),
        (
            "Failed(buffer: partial, proposal: second)",
            "Failed(buffer: partial, proposal: Token(number: 1))",
        ),
        (
            "    ended: drain(buffer)\n",
            "    ended: Ended(zero: 0, one: 0)\n",
        ),
        ("proposal_number: end_token(proposal)", "proposal_number: 1"),
    ] {
        let mutant = mutate(BUILD, before, after);
        assert_eq!(
            check("example.build", &mutant, &[&buffer, &lifecycle]).unwrap_err(),
            SourceCheckError::Unconsumed,
            "{mutant}"
        );
    }
}

#[test]
fn scalar_observers_reject_swapped_cleanup_receipts() {
    let buffer = buffer();
    let lifecycle = lifecycle(&buffer);
    let source = format!(
        "{CLIENT}
observe(first in 1 to 16, second in 1 to 16) in 0 to 16 and = second:
    Ended(zero, one) := both(first, second)
    return one
"
    );
    check("example.observer", &source, &[&buffer, &lifecycle]).unwrap();
    let mutant = mutate(
        LIFECYCLE,
        "return Ended(zero, one)",
        "return Ended(zero: one, one: zero)",
    );
    let wrong = check("example.lifecycle", &mutant, &[&buffer]).unwrap();
    assert_eq!(
        check("example.observer", &source, &[&buffer, &wrong]).unwrap_err(),
        SourceCheckError::Unproved
    );
}

#[test]
fn scalar_observers_reject_fabricated_failure_receipts_even_after_valid_ending() {
    let buffer = buffer();
    let lifecycle = lifecycle(&buffer);
    let build = check("example.build", BUILD, &[&buffer, &lifecycle]).unwrap();
    let source = format!(
        "{BUILD_CLIENT}
observe(first in 1 to 16, second in 1 to 16) in 1 to 16 and = second:
    Aborted(ended, proposal_number) := failed(first, second)
    Ended(zero, one) := ended
    return proposal_number
"
    );
    check("example.observer", &source, &[&buffer, &lifecycle, &build]).unwrap();
    let mutant = mutate(
        BUILD,
        "return Aborted(ended, proposal_number)",
        "return Aborted(ended, proposal_number: 1)",
    );
    let wrong = check("example.build", &mutant, &[&buffer, &lifecycle]).unwrap();
    assert_eq!(
        check("example.observer", &source, &[&buffer, &lifecycle, &wrong]).unwrap_err(),
        SourceCheckError::Unproved
    );
}
