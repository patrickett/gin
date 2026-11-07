//! Runtime-variant source checking, ownership, and body-free module evidence.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, SourceType, check_module};

const LIBRARY: &str = include_str!("../examples/runtime-slots.gin");
const CLIENT: &str = include_str!("../examples/runtime-slots-client.gin");
const SIMPLE: &str = "Slot(element) is Empty or Full(element)
";

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid sum/case source syntax");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    result
}

fn library() -> CheckedSourceModule {
    check("example.slots", LIBRARY, &[]).expect("real runtime-slot bodies verify")
}

fn rejects(body: &str, expected: SourceCheckError) {
    let source = format!("{SIMPLE}{body}");
    let error = check("example.invalid", &source, &[]).unwrap_err();
    assert_eq!(error, expected, "{source}");
}

#[test]
fn runtime_slots_and_the_separate_roundtrip_client_are_checked() {
    let library = library();
    let client = check("example.client", CLIENT, &[&library]).unwrap();
    assert_eq!(library.checked().functions.len(), 7);
    assert_eq!(client.checked().functions[0].name, "roundtrip");
}

#[test]
fn sum_inspection_data_retains_the_alias_and_its_nominal_members() {
    let library = library();
    let checked = library.checked();
    let slot = checked
        .aggregates
        .iter()
        .find(|schema| schema.name == "Slot")
        .unwrap();
    assert_eq!(slot.parameters, ["element"]);
    assert!(slot.fields.is_empty());
    assert_eq!(slot.variants.len(), 2);
    let take = checked
        .functions
        .iter()
        .find(|f| f.name == "take_slot")
        .unwrap();
    assert!(matches!(take.parameters[0], SourceType::Sum { .. }));
    assert!(matches!(take.result, SourceType::Sum { .. }));
}

#[test]
fn the_subject_is_consumed_once_even_when_it_is_a_function_call() {
    let source = format!(
        "{SIMPLE}
identity(element Type, slot Slot(element)) Slot(element): return slot
rebuild(element Type, slot Slot(element)) Slot(element):
    return when identity(element, slot) is
        Empty then Empty
        Full(value) then Full(value)
"
    );
    check("example.once", &source, &[]).unwrap();
}

#[test]
fn missing_extraneous_and_duplicate_cases_are_semantic_errors() {
    rejects(
        "bad(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Empty
",
        SourceCheckError::Type,
    );
    // An arm names a variant of the subject's union; a type of that name is not one.
    rejects(
        "Other has
bad(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Empty
        Full(value) then Full(value)
        Other then Empty
",
        SourceCheckError::Type,
    );
    rejects(
        "bad(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Empty
        Full(value) then Full(value)
        Empty then Empty
",
        SourceCheckError::Duplicate,
    );
}

#[test]
fn variant_field_arity_and_duplicate_binders_are_checked() {
    rejects(
        "bad(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty(value) then Empty
        Full(value) then Full(value)
",
        SourceCheckError::Type,
    );
    let source = "#unrestricted
Code is in 0 to 9
Choice is None or Pair(first Code, second Code)
bad(choice Choice) Choice:
    return when choice is
        None then None
        Pair(value, value) then Pair(value, second: value)
";
    let error = check("example.binders", source, &[]).unwrap_err();
    assert_eq!(error, SourceCheckError::Duplicate);
}

#[test]
fn a_live_payload_cannot_disappear_on_one_outcome() {
    rejects(
        "bad(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Empty
        Full(value) then Empty
",
        SourceCheckError::Unconsumed,
    );
}

#[test]
fn branch_payloads_and_outer_owners_cannot_be_duplicated() {
    rejects(
        "Pair(element) has first element, second element
Result(element) is Empty or Pair(Pair(element))
bad(element Type, slot Slot(element)) Result(element):
    return when slot is
        Empty then Empty
        Full(value) then Pair(Pair(first: value, second: value))
",
        SourceCheckError::UseAfterMove,
    );
    rejects(
        "Pair(element) has first element, second element
bad(element Type, slot Slot(element), other element) Pair(element):
    return when slot is
        Empty then Pair(first: other, second: other)
        Full(value) then Pair(first: value, second: other)
",
        SourceCheckError::UseAfterMove,
    );
}

#[test]
fn the_original_sum_cannot_be_reused_after_matching_it() {
    rejects(
        "bad(element Type, slot Slot(element)) Slot(element):
    rebuilt Slot(element): when slot is
        Empty then Empty
        Full(value) then Full(value)
    return slot
",
        SourceCheckError::UseAfterMove,
    );
}

#[test]
fn case_binders_do_not_leak_to_siblings_or_after_the_case() {
    rejects(
        "bad(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Full(value)
        Full(value) then Full(value)
",
        SourceCheckError::UnknownName,
    );
    rejects(
        "bad(element Type, slot Slot(element)) Slot(element):
    rebuilt Slot(element): when slot is
        Empty then Empty
        Full(value) then Full(value)
    return Full(value)
",
        SourceCheckError::UnknownName,
    );
}

#[test]
fn inactive_arm_bodies_do_not_hide_unbound_names_or_type_errors() {
    for (body, expected) in [
        ("missing", SourceCheckError::UnknownName),
        ("0", SourceCheckError::Type),
    ] {
        let source = format!(
            "{SIMPLE}
always_empty(element Type) Slot(element): return Empty
bad(element Type) Slot(element):
    return when always_empty(element) is
        Empty then Empty
        Full(value) then {body}
"
        );
        let error = check("example.inactive", &source, &[]).unwrap_err();
        assert_eq!(error, expected, "{source}");
    }
}

#[test]
fn imported_private_payloads_cannot_be_decomposed_by_case_patterns() {
    let library = library();
    let source = "use example.slots Slot, Holder
bad(element Type, holder Holder(element)) Slot(element):
    Holder(slot) := holder
    return slot
";
    let error = check("example.client", source, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::Privacy);
}

#[test]
fn sum_cases_support_nested_expression_and_indented_block_arms() {
    let source = format!(
        "{SIMPLE}
rebuild(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then
            return Empty
        Full(value) then
            saved: value
            again Slot(element): Full(saved)
            return when again is
                Empty then Empty
                Full(restored) then Full(restored)
"
    );
    check("example.nested", &source, &[]).unwrap();
}

#[test]
fn two_runtime_slots_account_for_all_four_active_variant_combinations() {
    let source = format!(
        "{SIMPLE}
Pair(element) has left Slot(element), right Slot(element)
with_empty(element Type, right Slot(element)) Pair(element):
    return when right is
        Empty then Pair(left: Empty, right: Empty)
        Full(value) then Pair(left: Empty, right: Full(value))
with_full(element Type, left_value element, right Slot(element)) Pair(element):
    return when right is
        Empty then Pair(left: Full(left_value), right: Empty)
        Full(right_value) then Pair(
            left: Full(left_value), right: Full(right_value))
rebuild_pair(element Type, left Slot(element), right Slot(element)) Pair(element):
    return when left is
        Empty then with_empty(element, right)
        Full(value) then with_full(element, value, right)
"
    );
    check("example.pair", &source, &[]).unwrap();
    let duplicated = source.replace(
        "right: Full(right_value)",
        "right: Full(left_value)",
    );
    assert_ne!(duplicated, source);
    assert_eq!(
        check("example.pair", &duplicated, &[]).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
}

#[test]
fn unit_case_statements_can_end_concrete_payloads() {
    let source = format!(
        "{SIMPLE}
Token has number in 0 to 16
no_op:
    return
finish_token(token Token):
    Token(number) := token
    return
finish_slot(slot Slot(Token)):
    when slot is
        Empty then no_op
        Full(token) then finish_token(token)
    return
"
    );
    check("example.endings", &source, &[]).unwrap();
}

#[test]
fn scalar_payload_identity_survives_imported_outcomes_and_consuming_cases() {
    let library = library();
    let source = format!(
        "{CLIENT}
Scalar has value in 0 to 16
observe_taken(payload Scalar) in 0 to 16:
    Scalar(number) := payload
    return number
observe(value in 0 to 16) in 0 to 16 and = value:
    full: fill_slot(Scalar, Scalar(value: value))
    rebuilt: roundtrip(Scalar, full)
    return when take_slot(Scalar, rebuilt) is
        Empty then value
        Taken(payload) then observe_taken(payload)
"
    );
    check("example.observer", &source, &[&library]).unwrap();
    let invalid = source.replace("    return number\n", "    return 0\n");
    assert_ne!(invalid, source);
    assert_eq!(
        check("example.observer", &invalid, &[&library]).unwrap_err(),
        SourceCheckError::Unproved
    );
}

#[test]
fn header_claims_are_checked_in_each_case_arm() {
    let source = "Number has value in 0 to 16
Choice is Empty or Number(Number)
read(choice Choice) out in 0 to 16 where out >= 0:
    return when choice is
        Empty then 0
        Number(number) then
            Number(value) := number
            return value
";
    check("example.claims", source, &[]).unwrap();
    let invalid = source.replace("out >= 0", "out < 0");
    assert_ne!(invalid, source);
    assert_eq!(
        check("example.claims", &invalid, &[]).unwrap_err(),
        SourceCheckError::Unproved
    );
}

const REPLACEMENT_OBSERVER: &str = "use example.slots Slot, Swapped, TakeResult
use example.slots empty_slot, fill_slot, take_slot, replace_slot
Scalar has number in 0 to 16
no_op:
    return
read_taken(value Scalar) in 0 to 16:
    Scalar(number) := value
    return number
finish_taken(value Scalar):
    Scalar(number) := value
    return
finish_slot(slot Slot(Scalar)):
    attempt: take_slot(Scalar, slot)
    when attempt is
        Empty then no_op
        Taken(value) then finish_taken(value)
    return
number_or(slot Slot(Scalar), fallback in 0 to 16) in 0 to 16:
    return when take_slot(Scalar, slot) is
        Empty then fallback
        Taken(value) then read_taken(value)
";

#[test]
fn replacement_outcomes_distinguish_the_previous_and_new_payloads() {
    let library = library();
    let swapped = LIBRARY.replace(
        "previous: Full(previous), current: next",
        "previous: Full(next), current: previous",
    );
    assert_ne!(swapped, LIBRARY);
    let swapped_library = check("example.slots", &swapped, &[]).unwrap();
    for (expected, ending) in [
        (
            "previous",
            "    finish_taken(new_value)\n    return number_or(old, previous)\n",
        ),
        (
            "next",
            "    number: read_taken(new_value)\n    finish_slot(old)\n    return number\n",
        ),
    ] {
        let source = format!(
            "{REPLACEMENT_OBSERVER}
observe(previous in 0 to 16, next in 0 to 16) in 0 to 16 and = {expected}:
    before: fill_slot(Scalar, Scalar(number: previous))
    Swapped(old, new_value) := replace_slot(Scalar, before, Scalar(number: next))
{ending}"
        );
        check("example.observer", &source, &[&library]).unwrap();
        let error = check("example.observer", &source, &[&swapped_library]).unwrap_err();
        assert_eq!(error, SourceCheckError::Unproved, "{source}");
    }
}

#[test]
fn aliases_cannot_change_the_payloads_nominal_type() {
    rejects(
        "First has number in 0 to 16
Second has number in 0 to 16
bad(slot Slot(First)) Slot(Second):
    return when slot is
        Empty then Empty
        Full(value) then Full(value)
",
        SourceCheckError::Type,
    );
}

#[test]
fn recursive_or_overlapping_sum_shapes_are_rejected() {
    let recursive = "Loop is Empty or Next(Loop)\n";
    assert_eq!(
        check("example.invalid", recursive, &[]).unwrap_err(),
        SourceCheckError::Type
    );
    for source in [
        "Bad(element) is Full(element) or Full(element)\n",
        "First has\nSecond has\nBad is Full(First) or Full(Second)\n",
    ] {
        let error = check("example.invalid", source, &[]).unwrap_err();
        assert_eq!(error, SourceCheckError::Duplicate, "{source}");
    }
}

#[test]
fn nested_unions_are_matched_through_their_wrapper_variant() {
    let source = format!(
        "{SIMPLE}
Outcome(element) is Done(Slot(element)) or Failed
rebuild_inner(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Empty
        Full(value) then Full(value)
rebuild(element Type, outcome Outcome(element)) Outcome(element):
    return when outcome is
        Done(inner) then Done(rebuild_inner(element, inner))
        Failed then Failed
"
    );
    check("example.aliases", &source, &[]).unwrap();
    let overlapping = source.replace(
        "Outcome(element) is Done(Slot(element)) or Failed",
        "Outcome(element) is Done(Slot(element)) or Done(element)",
    );
    assert_ne!(overlapping, source);
    assert_eq!(
        check("example.aliases", &overlapping, &[]).unwrap_err(),
        SourceCheckError::Duplicate
    );
}

#[test]
fn closed_sum_fields_do_not_depend_on_local_constructor_context() {
    let prefix = "Small is A or B
Big is A or B or C
always_a Small: return A
";
    for (declaration, function) in [
        (
            "Holder has item Small\n",
            "f Holder:\n    h: Holder(item: always_a)\n    return h\n",
        ),
        (
            "Holder(element) has value element, item Small\n",
            "f(element Type, value element) Holder(element):
    h: Holder(value: value, item: always_a)
    return h
",
        ),
    ] {
        let source = format!("{prefix}{declaration}{function}");
        check("example.inferred", &source, &[]).unwrap();
        let context = if declaration.starts_with("Holder(element)") {
            "h Holder(element):"
        } else {
            "h Holder:"
        };
        let annotated = source.replace("h:", context);
        assert_ne!(annotated, source);
        check("example.annotated", &annotated, &[]).unwrap();
    }
    for binding in ["h:", "h Holder:"] {
        let source = format!(
            "{prefix}Holder has item Small
bad(value Big) Holder:
    {binding} Holder(item: value)
    return h
"
        );
        let error = check("example.incompatible", &source, &[]).unwrap_err();
        assert_eq!(error, SourceCheckError::Type, "{source}");
    }
}

#[test]
fn changing_names_does_not_change_sum_or_case_capabilities() {
    fn rename(text: &str) -> String {
        text.replace("example.slots", "unrelated.choices")
            .replace("TakeResult", "Outcome")
            .replace("Slot", "Position")
            .replace("Taken", "Removed")
            .replace("Empty", "Vacant")
            .replace("Full", "Occupied")
            .replace("Holder", "Envelope")
    }
    let library = check("unrelated.choices", &rename(LIBRARY), &[]).unwrap();
    let client = check("another.client", &rename(CLIENT), &[&library]).unwrap();
    assert_eq!(library.checked().functions.len(), 7);
    assert_eq!(client.checked().functions.len(), 1);
}

#[test]
fn sum_identity_and_payloads_survive_a_dependency_diamond() {
    let base = check("base.slots", LIBRARY, &[]).unwrap();
    let left_source = "use base.slots Slot, Holder, pack_slot, unpack_slot
Left(element) has holder Holder(element)
left_make(element Type, slot Slot(element)) Left(element):
    return Left(holder: pack_slot(element, slot))
left_take(element Type, left Left(element)) Slot(element):
    Left(holder) := left
    return unpack_slot(element, holder)
";
    let right_source = left_source
        .replace("Left", "Right")
        .replace("left", "right");
    let left = check("bridge.left", left_source, &[&base]).unwrap();
    let right = check("bridge.right", &right_source, &[&base]).unwrap();
    let client = "use base.slots Slot
use bridge.left left_make, left_take
use bridge.right right_make, right_take
roundtrip(element Type, slot Slot(element)) Slot(element):
    left: left_make(element, slot)
    right: right_make(element, left_take(element, left))
    return right_take(element, right)
";
    check("example.diamond", client, &[&base, &left, &right]).unwrap();
    let duplicate = client.replace(
        "right_make(element, left_take(element, left))",
        "right_make(element, slot)",
    );
    assert_ne!(duplicate, client);
    let error = check("example.diamond", &duplicate, &[&base, &left, &right]).unwrap_err();
    assert_eq!(error, SourceCheckError::UseAfterMove);
}

#[test]
fn malformed_manual_case_tokens_reject_without_panicking_or_mutating_source() {
    use ast::{Expr, Item, Statement};

    let text = format!(
        "{SIMPLE}
rebuild(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Empty
        Full(value) then Full(value)
"
    );
    let mut source = parser::parse_strict(&text).unwrap();
    let Item::Binding(binding) = &mut source.items.last_mut().unwrap().value else {
        panic!("rebuild function");
    };
    let Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value else {
        panic!("body");
    };
    let Statement::Return(ret) = &mut block.statements[0].value else {
        panic!("return");
    };
    let Expr::Match(case) = &mut ret.value.as_mut().unwrap().value else {
        panic!("case expression");
    };
    case.arms.value[0].then_keyword = case.when_keyword;
    let before = source.clone();
    let result = std::panic::catch_unwind(|| {
        check_module("example.manual", &source, &[], Limits::default())
    });
    assert_eq!(result.unwrap().unwrap_err(), SourceCheckError::Malformed);
    assert_eq!(source, before);
}

#[test]
fn sum_work_and_flow_limits_reject_instead_of_dropping_a_variant() {
    let source = parser::parse_strict(LIBRARY).unwrap();
    let before = source.clone();
    for limits in [
        Limits {
            work_max: 0,
            ..Limits::default()
        },
        Limits {
            flow_work_max: 0,
            ..Limits::default()
        },
    ] {
        let error = check_module("example.limited", &source, &[], limits).unwrap_err();
        assert_eq!(error, SourceCheckError::Budget);
        assert_eq!(source, before);
    }
}
