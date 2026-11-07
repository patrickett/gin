//! Consuming, numeric-indexed two-slot values; no raw storage or mut extraction.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, SourceType, check_module};

const SLOTS: &str = include_str!("../examples/runtime-slots.gin");
const BUFFER: &str = include_str!("../examples/indexed-buffer.gin");
const CLIENT: &str = include_str!("../examples/indexed-buffer-client.gin");
const IMPORTS: &str = "use example.buffers Buffer, InsertResult, TakeResult, CloseResult
use example.buffers empty_buffer, insert_at, take_at, close_buffer
";

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid indexed-buffer source");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    result
}

fn library_from(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let slots = check("example.slots", SLOTS, &[]).unwrap();
    check("example.buffers", text, &[&slots])
}

fn library() -> CheckedSourceModule {
    library_from(BUFFER).expect("indexed-buffer bodies verify under default limits")
}

#[test]
fn insert_one_empty_take_zero_extract_one_and_close_verify() {
    let library = library();
    let client = check("example.client", CLIENT, &[&library]).unwrap();
    assert_eq!(library.checked().functions.len(), 9);
    assert_eq!(client.checked().functions[0].name, "roundtrip");
}

#[test]
fn the_buffer_has_one_type_not_one_type_per_occupancy_pattern() {
    let library = library();
    let checked = library.checked();
    let buffer = checked
        .aggregates
        .iter()
        .find(|schema| schema.name == "Buffer")
        .unwrap();
    assert_eq!(buffer.parameters, ["element"]);
    assert_eq!(buffer.fields.len(), 2);
    assert!(buffer.fields.iter().all(|field| field.private));
    assert!(
        buffer
            .fields
            .iter()
            .all(|field| matches!(field.ty, SourceType::Sum { .. }))
    );
    let insert = checked
        .functions
        .iter()
        .find(|f| f.name == "insert_at")
        .unwrap();
    assert_eq!(insert.parameters.len(), 3);
    assert!(matches!(insert.parameters[0], SourceType::Aggregate { .. }));
    assert!(matches!(insert.parameters[1], SourceType::Integer { .. }));
    assert!(matches!(insert.result, SourceType::Sum { .. }));
}

const RECOVER_OCCUPIED: &str = "
Values(element) has previous element, next element
recover(element Type, previous element, next element) Values(element):
    buffer: empty_buffer(element)
    Inserted(before) := insert_at(element, buffer, 1, previous)
    Occupied(after_failure, returned_next) := insert_at(element, before, 1, next)
    Vacant(after_check) := take_at(element, after_failure, 0)
    Removed(after_take, returned_previous) := take_at(element, after_check, 1)
    Closed := close_buffer(element, after_take)
    return Values(previous: returned_previous, next: returned_next)
";

#[test]
fn occupied_insertion_returns_the_buffer_and_the_proposed_value() {
    let library = library();
    let source = format!("{IMPORTS}{RECOVER_OCCUPIED}");
    check("example.recovery", &source, &[&library]).unwrap();
}

#[test]
fn failure_preserves_the_old_payload_and_returns_the_correct_proposal() {
    let library = library();
    let swapped = BUFFER.replace(
        "Full(previous)), value)",
        "Full(value)), value: previous)",
    );
    assert_ne!(swapped, BUFFER);
    let swapped_library = library_from(&swapped).unwrap();
    for (expected, returned) in [("previous", "old_number"), ("next", "new_number")] {
        let source = format!(
            "{IMPORTS}{RECOVER_OCCUPIED}
Scalar has number in 2 to 16
observe(previous in 2 to 16, next in 2 to 16) in 2 to 16 and = {expected}:
    Values(old, proposed) := recover(
        Scalar, Scalar(number: previous), Scalar(number: next))
    Scalar(old_number) := old
    Scalar(new_number) := proposed
    return {returned}
"
        );
        check("example.observer", &source, &[&library]).unwrap();
        let error = check("example.observer", &source, &[&swapped_library]).unwrap_err();
        assert_eq!(error, SourceCheckError::Unproved, "{source}");
    }
}

#[test]
fn an_empty_take_does_not_change_the_other_position() {
    let wrong_index = BUFFER.replace(
        "0 then take_zero(element, buffer)",
        "0 then take_one(element, buffer)",
    );
    assert_ne!(wrong_index, BUFFER);
    let library = library_from(&wrong_index).unwrap();
    let error = check("example.client", CLIENT, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::Type);
}

#[test]
fn repeated_runtime_index_aliases_and_identity_calls_stay_correlated() {
    let library = library();
    let source = format!(
        "{IMPORTS}
identity_index(index in 0 to 1) in 0 to 1: return index
roundtrip(element Type, value element, index in 0 to 1) element:
    buffer: empty_buffer(element)
    Inserted(after_insert) := insert_at(element, buffer, index, value)
    saved: index
    Removed(after_take, returned) := take_at(element, after_insert, identity_index(saved))
    Closed := close_buffer(element, after_take)
    return returned
"
    );
    check("example.correlated", &source, &[&library]).unwrap();
}

#[test]
fn two_independent_indices_are_not_assumed_equal() {
    let library = library();
    let source = format!(
        "{IMPORTS}
bad(element Type, value element, first in 0 to 1, second in 0 to 1) element:
    buffer: empty_buffer(element)
    Inserted(after_insert) := insert_at(element, buffer, first, value)
    Removed(after_take, returned) := take_at(element, after_insert, second)
    Closed := close_buffer(element, after_take)
    return returned
"
    );
    let error = check("example.independent", &source, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::Type);
}

#[test]
fn invalid_literal_and_unchecked_wider_index_domains_reject() {
    let library = library();
    for index in ["2", "index"] {
        let source = format!(
            "{IMPORTS}
bad(element Type, buffer Buffer(element),
    value element, index in 0 to 2) InsertResult(element):
    return insert_at(element, buffer, {index}, value)
"
        );
        let error = check("example.bounds", &source, &[&library]).unwrap_err();
        assert_eq!(error, SourceCheckError::Type, "{source}");
    }
}

#[test]
fn a_nonempty_close_returns_the_owned_buffer_instead_of_ending_its_payload() {
    let library = library();
    let source = format!(
        "{IMPORTS}
recover(element Type, value element) element:
    buffer: empty_buffer(element)
    Inserted(before_close) := insert_at(element, buffer, 1, value)
    StillFull(after_close) := close_buffer(element, before_close)
    Removed(empty, returned) := take_at(element, after_close, 1)
    Closed := close_buffer(element, empty)
    return returned
"
    );
    check("example.nonempty", &source, &[&library]).unwrap();
    let invalid = source.replace("StillFull(after_close)", "Closed");
    let error = check("example.nonempty", &invalid, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::Type);
}

#[test]
fn both_occupied_positions_survive_close_refusal_and_independent_extraction() {
    let library = library();
    let source = format!(
        "{IMPORTS}
Values(element) has zero element, one element
recover(element Type, zero element, one element) Values(element):
    buffer: empty_buffer(element)
    Inserted(first) := insert_at(element, buffer, 0, zero)
    Inserted(both) := insert_at(element, first, 1, one)
    StillFull(restored) := close_buffer(element, both)
    Removed(after_zero, returned_zero) := take_at(element, restored, 0)
    Removed(empty, returned_one) := take_at(element, after_zero, 1)
    Closed := close_buffer(element, empty)
    return Values(zero: returned_zero, one: returned_one)
"
    );
    check("example.both", &source, &[&library]).unwrap();
}

#[test]
fn failed_operations_cannot_drop_the_proposal_or_reuse_an_old_buffer_owner() {
    let library = library();
    let dropped = format!("{IMPORTS}{RECOVER_OCCUPIED}").replace(
        "return Values(previous: returned_previous, next: returned_next)",
        "return returned_previous",
    );
    let dropped = dropped.replace(") Values(element):", ") element:");
    let error = check("example.drop", &dropped, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::Unconsumed);
    let reused = CLIENT.replace(
        "take_at(element, after_insert, 0)",
        "take_at(element, buffer, 0)",
    );
    assert_ne!(reused, CLIENT);
    let error = check("example.reuse", &reused, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::UseAfterMove);
}

#[test]
fn zero_field_payloads_keep_their_obligation_after_indexed_extraction() {
    let library = library();
    let source = format!(
        "{CLIENT}
Marker has
finish:
    returned: roundtrip(Marker, Marker)
    Marker := returned
    return
"
    );
    check("example.empty_payload", &source, &[&library]).unwrap();
    let dropped = source.replace("    Marker := returned\n", "");
    assert_ne!(dropped, source);
    let error = check("example.empty_payload", &dropped, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::Unconsumed);
}
