//! Fixed-extent owned values and indexed child transfer, not raw backing storage.
use typecheck::{
    CheckedSourceInstanceHead, CheckedSourceModule, Limits, SourceCheckError, SourceType,
    check_module,
};

const LIBRARY: &str = include_str!("../examples/fixed-arrays.gin");
const CLIENT: &str = include_str!("../examples/fixed-arrays-client.gin");

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid fixed-array source");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    result
}

fn library() -> CheckedSourceModule {
    check("library.arrays", LIBRARY, &[]).expect("body-checked array library")
}

#[test]
fn owned_arrays_construct_swap_and_unpack_through_checked_interfaces() {
    let library = library();
    check("app.client", CLIENT, &[&library]).unwrap();
    for (returned, expected) in [("left_number", "second"), ("right_number", "first")] {
        let client = format!(
            "use library.arrays Pair, from_values, swap, take_both, pack, unpack
Scalar has number in 0 to 16
observe(first in 0 to 16, second in 0 to 16) in 0 to 16 and = {expected}:
    values: from_values(Scalar, Scalar(number: first), Scalar(number: second))
    held: pack(Scalar, values)
    exchanged: swap(Scalar, unpack(Scalar, held))
    Pair(left, right) := take_both(Scalar, exchanged)
    Scalar(left_number) := left
    Scalar(right_number) := right
    return {returned}
"
        );
        check("app.observe", &client, &[&library]).unwrap();
    }
}

#[test]
fn structural_array_heads_are_distinct_from_nominal_schemas() {
    let library = library();
    let checked = library.checked();
    let swap = checked
        .functions
        .iter()
        .find(|item| item.name == "swap")
        .unwrap();
    let SourceType::Array { id } = swap.result else {
        panic!("structural array result");
    };
    let instance = &checked.instances[id as usize];
    assert_eq!(
        instance.head,
        CheckedSourceInstanceHead::Array { extent: 2 }
    );
    assert_eq!(instance.arguments, [SourceType::Generic { index: 0 }]);
    assert!(
        checked
            .instances
            .iter()
            .any(|item| matches!(item.head, CheckedSourceInstanceHead::Nominal { .. }))
    );
}

#[test]
fn an_ownership_conserving_wrong_permutation_fails_the_identity_observer() {
    let mutant = LIBRARY.replace("return [second, first]", "return [first, second]");
    assert_ne!(mutant, LIBRARY);
    let library = check("library.arrays", &mutant, &[]).unwrap();
    let client = "
use library.arrays Pair, from_values, swap, take_both
Scalar has number in 0 to 16
observe(first in 0 to 16, second in 0 to 16) in 0 to 16 and = second:
    values: from_values(Scalar, Scalar(number: first), Scalar(number: second))
    Pair(left, right) := take_both(Scalar, swap(Scalar, values))
    Scalar(left_number) := left
    Scalar(right_number) := right
    return left_number
";
    assert_eq!(
        check("app.observe", client, &[&library]).unwrap_err(),
        SourceCheckError::Unproved
    );
}

#[test]
fn moved_slots_and_partial_whole_values_are_not_reusable() {
    for body in [
        "first: values.[0]\n    second: values.[0]\n    return [first, second]",
        "first: values.[0]\n    return values",
        "moved: values\n    first: values.[0]\n    return moved",
        "first: values.[0]\n    second: values.[1]\n    return [first, first]",
    ] {
        let text = format!("bad(element Type, values [element; 2]) [element; 2]:\n    {body}\n");
        assert_eq!(
            check("bad.moves", &text, &[]).unwrap_err(),
            SourceCheckError::UseAfterMove,
            "{text}"
        );
    }
}

#[test]
fn extraction_does_not_forget_the_other_slot_or_an_extracted_owner() {
    for text in [
        "bad(element Type, values [element; 2]) element: return values.[0]\n",
        concat!(
            "bad(element Type, values [element; 2]) element:\n",
            "    first: values.[0]\n",
            "    second: values.[1]\n",
            "    return first\n",
        ),
    ] {
        assert_eq!(
            check("bad.drop", text, &[]).unwrap_err(),
            SourceCheckError::Unconsumed
        );
    }
}

#[test]
fn fixed_extents_and_known_index_bounds_are_checked() {
    for text in [
        "bad(element Type, values [element; 2]) element: return values.[2]\n",
        "bad(element Type, values [element; 2]) element: return values.[-1]\n",
        "bad(element Type, value element) [element; 2]: return [value]\n",
    ] {
        assert_eq!(
            check("bad.bounds", text, &[]).unwrap_err(),
            SourceCheckError::Type
        );
    }
}

#[test]
fn closed_large_arrays_forward_without_expanding_their_elements() {
    for extent in [0, 2, 64, 65_536] {
        let text = format!(
            "forward(element Type, values [element; {extent}]) [element; {extent}]: return values\n"
        );
        let checked = check("library.forward", &text, &[]).unwrap();
        assert_eq!(checked.checked().functions.len(), 1);
    }
}

#[test]
fn an_unknown_owned_index_does_not_silently_choose_a_slot() {
    let text = "
bad(element Type, values [element; 2], index in 0 to 1) element:
    return values.[index]
";
    // Still refused, and not by choosing a slot: an element that may be linear cannot be copied out of
    // an owned array by a computed index (it was `Unsupported` before the index of an owned array of a
    // known length could be computed; now the refusal names the reason).
    assert_eq!(
        check("bad.unknown_index", text, &[]).unwrap_err(),
        SourceCheckError::MoveOutOfBorrow
    );
    // An unrestricted element is read at a proved index, and an unproved one is refused.
    let numbers = "
#unrestricted
Small is in 0 to 9

read(values mut [Small; 2], index in 0 to 1) Small:
    return values.[index]
";
    let wide = numbers.replace("index in 0 to 1", "index in 0 to 2");
    assert!(check("ok.unknown_index", numbers, &[]).is_ok(), "{:?}", check("ok.unknown_index", numbers, &[]));
    assert!(check("past.unknown_index", &wide, &[]).is_err());
}

#[test]
fn numeric_case_facts_select_exact_places_and_keep_imported_origins() {
    let ordered = format!(
        "{LIBRARY}
ordered(element Type, values [element; 2], index in 0 to 1) Pair(element):
    return when index is
        0 then
            first: values.[index]
            second: values.[1]
            return Pair(left: first, right: second)
        1 then
            first: values.[index]
            second: values.[0]
            return Pair(left: first, right: second)
"
    );
    let library = check("library.arrays", &ordered, &[]).unwrap();
    let client = "
use library.arrays Pair, from_values, ordered
Scalar has number in 0 to 16
observe(index in 0 to 1, first in 0 to 16, second in 0 to 16) out in 0 to 16 where out >= 0, out <= 16:
    return when index is
        0 then
            values: from_values(Scalar, Scalar(number: first), Scalar(number: second))
            Pair(left, right) := ordered(Scalar, values, index)
            Scalar(left_number) := left
            Scalar(right_number) := right
            return left_number
        1 then
            values: from_values(Scalar, Scalar(number: first), Scalar(number: second))
            Pair(left, right) := ordered(Scalar, values, index)
            Scalar(left_number) := left
            Scalar(right_number) := right
            return left_number
";
    check("app.ordered", client, &[&library]).unwrap();
}

/// Decision (3): an array has no methods, so a record around one ends its elements with a loop
/// it writes itself. The loop is accepted because the checker proves its body ends the element
/// it is given (`drain_loops.rs`, tests `drained_arrays_and_domain_claims.rs`), not because it
/// parses: a body that leaves the element is `Unconsumed` there. This used to be `Unsupported`.
#[test]
fn a_cleanup_loop_over_an_array_is_accepted_when_it_ends_every_element() {
    let text = "
Token has number in 0 to 16
consume(value Token):
    Token(number) := value
    return
drain(values [Token; 2]):
    for value in values do
        consume(value)
    return
";
    check("future.loop_cleanup", text, &[]).unwrap();
}

const TRANSPOSE: &str = "
transpose(element Type, values [[element; 2]; 2]) [[element; 2]; 2]:
    row0: values.[0]
    row1: values.[1]
    a: row0.[0]
    b: row0.[1]
    c: row1.[0]
    d: row1.[1]
    return [[a, c], [b, d]]
";

#[test]
fn nested_array_positions_keep_their_distinct_origins() {
    let client = "
use library.matrix transpose
Scalar has number in 0 to 16
observe(a in 0 to 16, b in 0 to 16, c in 0 to 16, d in 0 to 16) in 0 to 16 and = b:
    matrix: [[Scalar(number: a), Scalar(number: b)], [Scalar(number: c), Scalar(number: d)]]
    rows: transpose(Scalar, matrix)
    first: rows.[0]
    second: rows.[1]
    t00: first.[0]
    t01: first.[1]
    t10: second.[0]
    t11: second.[1]
    Scalar(n00) := t00
    Scalar(n01) := t01
    Scalar(n10) := t10
    Scalar(n11) := t11
    return n10
";
    let library = check("library.matrix", TRANSPOSE, &[]).unwrap();
    check("app.matrix", client, &[&library]).unwrap();
    let wrong = TRANSPOSE.replace("return [[a, c], [b, d]]", "return [[a, b], [c, d]]");
    assert_ne!(wrong, TRANSPOSE);
    let wrong = check("library.matrix", &wrong, &[]).unwrap();
    assert_eq!(
        check("app.matrix", client, &[&wrong]).unwrap_err(),
        SourceCheckError::Unproved
    );
}

#[test]
fn selecting_a_sum_child_does_not_require_inspecting_its_sibling() {
    let slots = check(
        "example.slots",
        include_str!("../examples/runtime-slots.gin"),
        &[],
    )
    .unwrap();
    let text = "
use example.slots Slot, rebuild_slot
rebuild_first(element Type, values [Slot(element); 2]) [Slot(element); 2]:
    first: values.[0]
    second: values.[1]
    return [rebuild_slot(element, first), second]
";
    check("library.sum_arrays", text, &[&slots]).unwrap();
}

const ARRAY_VIEWS: &str = "
Big is A or B or C
Held has values [Big; 1]
Box(element) has value element
make [Big; 1]: return [A]
widen(values [Big; 1]) [Big; 1]: return values
pack(values [Big; 1]) Held: return Held(values)
identity(element Type, value element) element: return value
";

#[test]
fn declared_element_coverage_survives_array_views_and_imported_summaries() {
    let library = check("library.views", ARRAY_VIEWS, &[]).unwrap();
    let imports = "use library.views Big, Held, Box, make, widen, pack, identity\n";
    for binding in [
        "values: widen(make)",
        "values [Big; 1]: make",
        "Held(values) := pack(make)",
        "values: identity([Big; 1], widen(make))",
        "box: Box(value: widen(make))\n    Box(values) := box",
    ] {
        let observer = format!(
            "observe Big:
    {binding}
    value: values.[0]
    return when value is
        A then A
        B then A
"
        );
        for (prefix, dependencies) in [(ARRAY_VIEWS, Vec::new()), (imports, vec![&library])] {
            let incomplete = format!("{prefix}{observer}");
            assert_eq!(
                check("app.views", &incomplete, &dependencies).unwrap_err(),
                SourceCheckError::Type,
                "{incomplete}"
            );
            let complete = format!("{incomplete}        C then A\n");
            check("app.views", &complete, &dependencies).unwrap();
        }
    }
}

#[test]
fn nested_array_views_preserve_the_declared_inner_element_domain() {
    let declarations = "
Big is A or B or C
make [[Big; 1]; 1]: return [[A]]
widen(values [[Big; 1]; 1]) [[Big; 1]; 1]: return values
";
    let observer = "
observe Big:
    values: widen(make)
    row: values.[0]
    value: row.[0]
    return when value is
        A then A
        B then A
";
    let incomplete = format!("{declarations}{observer}");
    assert_eq!(
        check("app.nested_views", &incomplete, &[]).unwrap_err(),
        SourceCheckError::Type
    );
    check(
        "app.nested_views",
        &format!("{incomplete}        C then A\n"),
        &[],
    )
    .unwrap();
}
