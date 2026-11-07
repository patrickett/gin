//! Real source-body evidence for bounded, statically known slot states.
//! No allocator, raw layout, dynamic union, or borrowed extraction is modeled here.
use typecheck::{
    CheckedSourceInstanceHead, CheckedSourceModule, Limits, SourceCheckError, SourceType,
    check_module,
};

const LIBRARY: &str = include_str!("../examples/value-slots.gin");
const CLIENT: &str = include_str!("../examples/value-slots-client.gin");
const IMPORTS: &str = "use example.slots EmptySlot, FullSlot, Extracted, SlotPair
use example.slots empty_slot, fill_slot, take_slot, replace_slot
use example.slots empty_pair, put_left, put_right, take_left, take_right, finish_empty_pair
";

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("existing Gin syntax");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    result
}

fn library() -> CheckedSourceModule {
    check("example.slots", LIBRARY, &[]).expect("slot bodies verify")
}

fn rejects(body: &str, expected: SourceCheckError) {
    let library = library();
    let text = format!("{IMPORTS}{body}");
    let error = check("example.client", &text, &[&library]).unwrap_err();
    assert_eq!(error, expected, "{text}");
}

#[test]
fn real_slot_bodies_and_separate_nonprefix_reuse_client_verify() {
    let library = library();
    let client = check("example.client", CLIENT, &[&library]).unwrap();
    assert_eq!(library.checked().functions.len(), 10);
    assert_eq!(client.checked().functions[0].name, "cycle");
}

#[test]
fn an_empty_slot_cannot_supply_a_value() {
    rejects(
        "bad(element Type, slot EmptySlot) Extracted(element, EmptySlot):
    return take_slot(element, slot)
",
        SourceCheckError::Type,
    );
}

#[test]
fn a_full_slot_cannot_be_initialized_again() {
    rejects(
        "bad(element Type, slot FullSlot(element), next element) FullSlot(element):
    return fill_slot(element, slot, next)
",
        SourceCheckError::Type,
    );
}

#[test]
fn same_live_count_does_not_identify_the_live_position() {
    rejects(
        "bad(element Type, pair SlotPair(EmptySlot, FullSlot(element))) SlotPair(
        FullSlot(element), EmptySlot):
    return pair
",
        SourceCheckError::Type,
    );
}

#[test]
fn a_slot_and_its_extracted_payload_cannot_be_owned_twice() {
    rejects(
        "bad(element Type, slot FullSlot(element)) Extracted(element, EmptySlot):
    first: take_slot(element, slot)
    return take_slot(element, slot)
",
        SourceCheckError::UseAfterMove,
    );
    rejects(
        "bad(element Type, slot FullSlot(element)) FullSlot(element):
    Extracted(value, empty) := take_slot(element, slot)
    filled: fill_slot(element, empty, value)
    return fill_slot(element, empty_slot, value)
",
        SourceCheckError::UseAfterMove,
    );
}

#[test]
fn neither_an_occupied_slot_nor_an_extracted_value_can_be_abandoned() {
    rejects(
        "bad(element Type, slot FullSlot(element)): return
",
        SourceCheckError::Unconsumed,
    );
    rejects(
        "bad(element Type, slot FullSlot(element)) EmptySlot:
    Extracted(value, empty) := take_slot(element, slot)
    return empty
",
        SourceCheckError::Unconsumed,
    );
}

#[test]
fn replacement_returns_the_previous_value_and_preserves_the_next() {
    let library = library();
    let client = format!(
        "{IMPORTS}
Replaced(element) has
    previous element
    next FullSlot(element)
replace(element Type, previous element, next element) Replaced(element):
    slot: fill_slot(element, empty_slot, previous)
    Extracted(old, updated) := replace_slot(element, slot, next)
    return Replaced(previous: old, next: updated)
"
    );
    check("example.client", &client, &[&library]).unwrap();
    let dropped = LIBRARY.replace(
        "return Extracted(value: previous, rest: FullSlot(value: next))",
        "return Extracted(value: next, rest: FullSlot(value: next))",
    );
    assert_ne!(dropped, LIBRARY);
    let error = check("example.slots", &dropped, &[]).unwrap_err();
    assert_eq!(error, SourceCheckError::UseAfterMove);
    rejects(
        "bad(element Type, slot FullSlot(element), next element) FullSlot(element):
    Extracted(old, updated) := replace_slot(element, slot, next)
    return updated
",
        SourceCheckError::Unconsumed,
    );
}

#[test]
fn replacement_old_and_updated_outputs_preserve_their_distinct_identities() {
    let library = library();
    let swapped = LIBRARY.replace(
        "return Extracted(value: previous, rest: FullSlot(value: next))",
        "return Extracted(value: next, rest: FullSlot(value: previous))",
    );
    assert_ne!(swapped, LIBRARY);
    let swapped_library = check("example.slots", &swapped, &[]).unwrap();
    for (expected, returned) in [("previous", "old_number"), ("next", "new_number")] {
        let source = format!(
            "{IMPORTS}
Scalar has value in 0 to 16
observe(previous in 0 to 16, next in 0 to 16) in 0 to 16 and = {expected}:
    slot: fill_slot(Scalar, empty_slot, Scalar(value: previous))
    Extracted(old, updated) := replace_slot(Scalar, slot, Scalar(value: next))
    Extracted(new, empty) := take_slot(Scalar, updated)
    EmptySlot := empty
    Scalar(old_number) := old
    Scalar(new_number) := new
    return {returned}
"
        );
        check("example.client", &source, &[&library]).unwrap();
        let error = check("example.client", &source, &[&swapped_library]).unwrap_err();
        assert_eq!(error, SourceCheckError::Unproved, "{source}");
    }
}

#[test]
fn a_generic_unknown_state_is_neither_known_empty_nor_known_full() {
    rejects(
        "bad(element Type, state Type, slot state) Extracted(element, EmptySlot):
    return take_slot(element, slot)
",
        SourceCheckError::Type,
    );
    rejects(
        "bad(element Type, state Type, slot state, value element) FullSlot(element):
    return fill_slot(element, slot, value)
",
        SourceCheckError::Type,
    );
}

#[test]
fn only_an_empty_pair_can_use_the_empty_ending_operation() {
    rejects(
        "bad(element Type, pair SlotPair(EmptySlot, FullSlot(element))):
    finish_empty_pair(pair)
    return
",
        SourceCheckError::Type,
    );
}

#[test]
fn private_representation_cannot_bypass_the_checked_transition_api() {
    rejects(
        "bad(element Type, value element) FullSlot(element):
    return FullSlot(value: value)
",
        SourceCheckError::Privacy,
    );
}

const OBSERVE: &str = "
Scalar has
    value in 0 to 16
observe(value in 0 to 16) in 0 to 16 and = value:
    pair: cycle(Scalar, Scalar(value: value), Scalar(value: 0))
    Values(first, second) := pair
    Scalar(returned) := first
    Scalar(other) := second
    return returned
";

#[test]
fn returned_payload_identity_survives_slot_transitions_and_imported_calls() {
    let library = library();
    let source = format!("{CLIENT}{OBSERVE}");
    check("example.client", &source, &[&library]).unwrap();
    let changed = source.replace("    return returned\n", "    return other\n");
    assert_ne!(changed, source);
    let error = check("example.client", &changed, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::Unproved);
}

#[test]
fn zero_field_nominal_payloads_still_have_to_be_accounted_for() {
    let library = library();
    let source = format!(
        "{CLIENT}
Marker has
finish:
    Values(first, second) := cycle(Marker, Marker, Marker)
    Marker := first
    Marker := second
    return
"
    );
    check("example.client", &source, &[&library]).unwrap();
    let changed = source.replace("    Marker := second\n", "");
    assert_ne!(changed, source);
    let error = check("example.client", &changed, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::Unconsumed);
}

fn rename(source: &str) -> String {
    source
        .replace("example.slots", "unrelated.cells")
        .replace("SlotPair", "Positions")
        .replace("EmptySlot", "Absent")
        .replace("FullSlot", "Present")
        .replace("Extracted", "Taken")
        .replace("fill_slot", "place_value")
        .replace("take_slot", "remove_value")
}

#[test]
fn state_and_operation_names_have_no_privileged_meaning() {
    let library = check("unrelated.cells", &rename(LIBRARY), &[]).unwrap();
    let client = check("another.client", &rename(CLIENT), &[&library]).unwrap();
    assert_eq!(library.checked().functions.len(), 10);
    assert_eq!(client.checked().functions.len(), 1);
}

#[test]
fn ordinary_sum_declarations_are_checked_without_changing_the_static_slot_model() {
    let source = "Slot(element) is Empty or Full(element)\n";
    let module = check("example.dynamic", source, &[]).unwrap();
    assert!(module.checked().functions.is_empty());
}

const CAPACITY_MAX: usize = 3;

fn cells_type(capacity: usize, mask: u8) -> String {
    assert!(capacity <= CAPACITY_MAX);
    assert!(mask < (1_u8 << capacity));
    if capacity == 0 {
        return "Cells0".into();
    }
    let states: Vec<_> = (0..capacity)
        .map(|index| {
            if mask & (1 << index) == 0 {
                "EmptySlot"
            } else {
                "FullSlot(element)"
            }
        })
        .collect();
    format!("Cells{capacity}({})", states.join(", "))
}

fn cells_declaration(capacity: usize) -> String {
    assert!(capacity <= CAPACITY_MAX);
    if capacity == 0 {
        return "Cells0 has\n".into();
    }
    let states: Vec<_> = (0..capacity).map(|i| format!("state_{i}")).collect();
    let mut source = format!("Cells{capacity}({}) has\n", states.join(", "));
    for index in 0..capacity {
        source.push_str(&format!("    slot_{index} state_{index}\n"));
    }
    source
}

fn cells_roundtrip(capacity: usize, mask: u8) -> String {
    let ty = cells_type(capacity, mask);
    let result = ty.replacen('(', "(\n        ", 1);
    let mut source = format!("roundtrip(element Type,\n          cells {ty}) {result}:\n");
    if capacity == 0 {
        source.push_str("    Cells0 := cells\n    return Cells0\n");
        return source;
    }
    let names: Vec<_> = (0..capacity).map(|i| format!("slot_{i}")).collect();
    source.push_str(&format!(
        "    Cells{capacity}({}) := cells\n",
        names.join(", ")
    ));
    let mut fields = Vec::new();
    for index in 0..capacity {
        if mask & (1 << index) == 0 {
            fields.push(format!("slot_{index}: slot_{index}"));
        } else {
            source.push_str(&format!(
                "    Extracted(value_{index}, empty_{index}) := take_slot(element, slot_{index})\n"
            ));
            source.push_str(&format!(
                "    restored_{index}: fill_slot(element, empty_{index}, value_{index})\n"
            ));
            fields.push(format!("slot_{index}: restored_{index}"));
        }
    }
    source.push_str(&format!(
        "    return Cells{capacity}({})\n",
        fields.join(", ")
    ));
    source
}

fn cells_make(capacity: usize, mask: u8) -> String {
    let result = cells_type(capacity, mask).replacen('(', "(\n        ", 1);
    let mut parameters = vec!["element Type".to_owned()];
    let mut fields = Vec::new();
    for index in 0..capacity {
        if mask & (1 << index) == 0 {
            fields.push(format!("slot_{index}: empty_slot"));
        } else {
            parameters.push(format!("value_{index} element"));
            fields.push(format!(
                "slot_{index}: fill_slot(element, empty_slot, value_{index})"
            ));
        }
    }
    let mut source = format!("make({}) {result}:\n", parameters.join(",\n     "));
    if capacity == 0 {
        source.push_str("    return Cells0\n");
    } else {
        source.push_str(&format!(
            "    return Cells{capacity}(\n        {})\n",
            fields.join(",\n        ")
        ));
    }
    source
}

fn checked_footprint(module: &CheckedSourceModule, name: &str, capacity: usize) -> u8 {
    assert!(capacity <= CAPACITY_MAX);
    let checked = module.checked();
    let function = checked.functions.iter().find(|f| f.name == name).unwrap();
    let SourceType::Aggregate { id } = function.result else {
        panic!("nominal cells result");
    };
    let instance = &checked.instances[id as usize];
    let CheckedSourceInstanceHead::Nominal { schema } = instance.head else {
        panic!("nominal cells instance");
    };
    assert_eq!(
        checked.aggregates[schema as usize].name,
        format!("Cells{capacity}")
    );
    assert_eq!(instance.arguments.len(), capacity);
    let mut mask = 0;
    for (index, ty) in instance.arguments.iter().enumerate() {
        let SourceType::Aggregate { id } = ty else {
            panic!("nominal slot state");
        };
        let state = &checked.instances[*id as usize];
        let CheckedSourceInstanceHead::Nominal { schema } = state.head else {
            panic!("nominal slot instance");
        };
        match checked.aggregates[schema as usize].name.as_str() {
            "EmptySlot" => assert!(state.arguments.is_empty()),
            "FullSlot" => {
                assert_eq!(state.arguments, [SourceType::Generic { index: 0 }]);
                mask |= 1 << index;
            }
            other => panic!("unexpected slot state: {other}"),
        }
    }
    mask
}

#[test]
fn all_fifteen_static_footprints_for_capacities_zero_through_three_verify() {
    let library = library();
    let mut cases = 0;
    let mut nonprefix = 0;
    for capacity in 0..=CAPACITY_MAX {
        for mask in 0..(1_u8 << capacity) {
            let source = format!(
                "{IMPORTS}{}{}{}",
                cells_declaration(capacity),
                cells_make(capacity, mask),
                cells_roundtrip(capacity, mask)
            );
            assert!(source.lines().all(|line| line.len() <= 100), "{source}");
            let module = check("example.footprint", &source, &[&library])
                .unwrap_or_else(|error| panic!("{error:?}\n{source}"));
            for name in ["make", "roundtrip"] {
                assert_eq!(checked_footprint(&module, name, capacity), mask);
            }
            cases += 1;
            if mask != (1_u8 << mask.count_ones()) - 1 {
                nonprefix += 1;
            }
        }
    }
    assert_eq!(cases, 15);
    assert_eq!(nonprefix, 5);
}
