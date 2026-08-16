//! Flow analysis tests for the ownership system.
//!
//! Covers the core rules of the linear value ownership model:
//!   1. Creation   — a linear value is tracked by the type system
//!   2. Transfer   — ownership moves when passed to a `eat` param
//!   3. Consumption— using a linear value fulfills the "use once" requirement
//!   4. No copying — implicit duplication is rejected (size-based: ≤ 16 bytes = Copy)
//!   5. No dropping— implicit discard is rejected (> 16 bytes = linear)
//!   6. Borrowing  — non-owning references (TakePtr/Deref) preserve ownership
//!   7. Aliasing   — aliases only under single-ownership-preserving rules
//!   8. Return     — linear value must be consumed or transferred before scope exit
//!
//! Tests are grouped by the diagnostic they expect.
//!
//! NOTE: gin_core modules are NOT loaded in these tests. User-defined types
//! resolve to Ty::Opaque (8 bytes, ≤ 16, Copy) when they can't be found.
//! LinValueNotConsumed flow tests are therefore limited — the copyability unit
//! tests in `test_copy_inference_*` directly verify the size-based rule.

use internment::Intern;
use typecheck::FileId;
use typecheck::transform::transform_file;

/// `Int` tag so ownership tests can type parameters without loading gin_core.
const INT_TAG: &str = "Int is in 1...400\n\n";

fn ownership_source(body: &str) -> String {
    format!("{INT_TAG}{body}")
}

/// Parse source, transform via typed AST pipeline, and collect diagnostics.
fn collect_ownership_diagnostics(source: &str) -> Vec<diagnostic::Diagnostic> {
    let ast = parser::cursor::TokenCursor::parse_source(&ownership_source(source));
    let typed = transform_file(ast.clone(), FileId(0));
    let mut diags = Vec::new();
    for (_, flaw) in typed.all_flaws() {
        diags.push(flaw.clone());
    }
    diags
}

#[test]
fn test_use_after_move_produces_diagnostic() {
    // Consume via `eat` at call site — subsequent use should produce UseOfMovedValue.
    let src = "\
drop(eat x Int) Int: 0
main:
    val: 42
    drop(eat val)
    result: val
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved_diag = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(
        has_moved_diag,
        "expected UseOfMovedValue diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_double_consume_produces_diagnostic() {
    // Two consecutive `eat` on the same value — second is UseOfMovedValue.
    let src = "\
drop(eat x Int) Int: 0
main:
    val: 42
    drop(eat val)
    drop(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(
        has_moved,
        "expected UseOfMovedValue for double consume, got: {:?}",
        diags
    );
}

#[test]
fn test_no_false_positive_use_after_move_copyable() {
    // Copyable (Int) value used normally — no UseOfMovedValue.
    let src = "\
main:
    val: 42
    result: val + 1
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved_diag = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved_diag, "unexpected UseOfMovedValue diagnostic");
}

#[test]
fn test_consume_via_eat_no_false_positive() {
    // `eat` at call site matched with `eat` param — no diagnostic.
    let src = "\
drop(eat x Int) Int: 0
main:
    val: 42
    drop(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved, "unexpected UseOfMovedValue: {:?}", diags);
}

#[test]
fn test_use_after_consume_via_eat_detected() {
    // Consume via `eat`, then use — should be an error.
    let src = "\
drop(eat x Int) Int: 0
main:
    val: 42
    drop(eat val)
    result: val
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(has_moved, "use after eat consume: {:?}", diags);
}

#[test]
fn test_default_owned_copy_argument_remains_alive() {
    // Passing a structurally Copy value by ownership copies it.
    let src = "\
read(x Int) Int: x
main:
    val: 42
    read(val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved, "owned Copy argument moved: {:?}", diags);
}

#[test]
fn test_default_owned_non_copy_argument_moves() {
    let src = "\
sink(pointer Pointer(Int)): eat pointer
main:
    value: 42
    pointer: @value
    sink(pointer)
    result: deref pointer
return 0
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        diags
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "type-use-of-moved-value"),
        "expected owned non-Copy argument to move, got: {:?}",
        diags
    );
}

#[test]
fn test_default_owned_copy_arguments_can_be_chained() {
    // Each ownership pass copies a structurally Copy value.
    let src = "\
add_one(x Int) Int: x + 1
double(y Int) Int: y * 2
main:
    val: 42
    s1: add_one(val)
    r: double(s1)
return r
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(
        !has_moved,
        "chained owned Copy arguments moved: {:?}",
        diags
    );
}

#[test]
fn test_move_in_when_arm_does_not_poison_sibling_arm() {
    let src = "\
sink(pointer Pointer(Int)) Int: eat pointer
select(flag Int, pointer Pointer(Int)) Int:
    when flag is
        0 then sink(pointer)
        else deref pointer
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        !diags.iter().any(|diagnostic| matches!(
            diagnostic.code.slug(),
            "type-use-of-moved-value" | "type-use-of-maybe-moved-value"
        )),
        "exclusive sibling arm saw moved value: {:?}",
        diags
    );
}

#[test]
fn test_move_on_one_when_path_is_maybe_moved_after_join() {
    let src = "\
sink(pointer Pointer(Int)) Int: eat pointer
select(flag Int, pointer Pointer(Int)) Int:
    when flag is
        0 then sink(pointer)
        else 0
    result: deref pointer
return result
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        diags
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "type-use-of-maybe-moved-value"),
        "expected maybe-moved diagnostic after branch join, got: {:?}",
        diags
    );
}

#[test]
fn test_loop_backedge_rejects_reusing_moved_value() {
    let src = "\
sink(pointer Pointer(Int)): eat pointer
repeat(pointer Pointer(Int)):
    while 1 is 1
        sink(pointer)
    loop
return
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        diags
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "type-use-of-moved-value"),
        "expected move error on a possible later iteration, got: {:?}",
        diags
    );
}

#[test]
fn test_reference_invalidated_on_one_when_path_is_maybe_invalid_after_join() {
    let src = "\
Cell has value Int
sink(cell Cell): eat cell
check(flag Int, cell Cell) Int:
    ref saved Cell: cell
    when flag is
        0 then sink(cell)
        else saved.value
    result: saved.value
return result
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        diags.iter().any(|diagnostic| {
            diagnostic.code.slug() == "type-use-of-maybe-invalidated-reference"
        }),
        "expected maybe-invalidated diagnostic after branch join, got: {:?}",
        diags
    );
    assert!(
        !diags
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "type-use-of-invalidated-reference"),
        "exclusive sibling arm saw invalidated reference: {:?}",
        diags
    );
}

#[test]
fn test_observe_then_consume() {
    let src = "\
read(ref x Int) Int: x
drop(eat y Int) Int: 0
main:
    val: 42
    read(ref val)
    drop(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved, "observe then consume: {:?}", diags);
}

#[test]
fn test_default_owned_copy_and_eat_arguments_can_be_mixed() {
    // Mix a default-owned Copy parameter with an explicitly consumed parameter.
    let src = "\
pair(a Int, eat b Int) Int: a + b
main:
    x: 10
    y: 20
    pair(x, eat y)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved, "mixed eat and bare: {:?}", diags);
}

#[test]
fn test_eat_param_returned_is_error() {
    // Returning a `eat` param is invalid (it's auto-dropped at scope exit).
    let src = "\
consume(eat x Int) Int: x
main:
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_return_consumed = diags
        .iter()
        .any(|d| d.code.slug() == "type-return-consumed-param");
    assert!(
        has_return_consumed,
        "expected ReturnConsumedParam, got: {:?}",
        diags
    );
}

#[test]
fn test_eat_param_returned_reports_consuming_parameter_escapes() {
    let src = "\
consume(eat x Int) Int: x
main:
    consume(eat 42)
    return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_escape = diags
        .iter()
        .any(|d| d.code.slug() == "type-consuming-parameter-escapes");
    assert!(
        has_escape,
        "expected consuming-parameter-escape diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_partial_record_destructure_discards_non_discardable_remainder() {
    let src = "
Point has head Int, tail Pointer(Int)
main:
    value: 42
    point := Point(head: value, tail: @value)
    Point(head: projected) := point
";
    let diags = collect_ownership_diagnostics(src);
    let has_remainder_error = diags
        .iter()
        .any(|d| d.code.slug() == "type-pattern-would-discard-value");
    assert!(
        has_remainder_error,
        "expected pattern discard diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_partial_record_destructure_with_nominal_remainder_reports_discard_error() {
    let src = "
Point has head Int, tail Pointer(Int)
main:
    ptr: @42
    point: Point(head: 1, tail: ptr)
    Point(head: value) := point
";
    let diags = collect_ownership_diagnostics(src);
    let has_remainder_error = diags
        .iter()
        .any(|d| d.code.slug() == "type-pattern-would-discard-value");
    assert!(
        has_remainder_error,
        "nominal remainder should fail discardability check: {:?}",
        diags
    );
}

#[test]
fn test_discardable_sum_with_nullary_alternative_is_allowed() {
    let src = "
Option(Int) is Some(Int) or None
main:
    val: None
    when val is
        Some(_) then 0
        None then 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_discard_error = diags
        .iter()
        .any(|d| d.code.slug() == "type-pattern-would-discard-value");
    assert!(
        !has_discard_error,
        "nullary-sum fallback should be discardable: {:?}",
        diags
    );
}

#[test]
fn test_sum_discardability_with_reachable_nominal_payload_and_proven_branch_is_allowed() {
    let src = "
ValueCell has value Pointer(Int)
CellMaybe is Some(ValueCell) or None
main:
    cell: @42
    val: Some(ValueCell(value: cell))
    when val is
        Some(_) then 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_discard_error = diags
        .iter()
        .any(|d| d.code.slug() == "type-pattern-would-discard-value");
    assert!(
        !has_discard_error,
        "active reachable arm should admit discardability: {:?}",
        diags
    );
}

#[test]
fn test_eat_argument_on_default_owned_param_is_error() {
    // `eat` does not match a default-owned parameter.
    let src = "\
read(x Int) Int: x
main:
    val: 42
    read(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_mode_mismatch = diags
        .iter()
        .any(|d| d.code.slug() == "type-consume-arg-on-owned-param");
    assert!(
        has_mode_mismatch,
        "expected consume-on-owned diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_eat_param_unused_is_valid() {
    // `eat` param that is never used in the body — auto-dropped, no diagnostic.
    let src = "\
leaky(eat x Int) Int: 0
main:
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_lin = diags
        .iter()
        .any(|d| d.code.slug() == "type-lin-value-not-consumed");
    assert!(
        !has_lin,
        "eat param unconsumed should be valid (auto-dropped), got: {:?}",
        diags
    );
}

#[test]
fn test_nominal_record_owned_param_not_consumed_reports_owned_value_not_consumed() {
    let src = "
Point has head Int, tail Pointer(Int)
consume(point Point) Int: 0
main:
    value: 42
    point: Point(head: value, tail: @value)
    consume(point)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_owned_not_consumed = diags
        .iter()
        .any(|d| d.code.slug() == "type-owned-value-not-consumed");
    assert!(
        has_owned_not_consumed,
        "expected owned value not consumed at callable exit: {:?}",
        diags
    );
}

#[test]
fn test_nominal_record_with_only_anonymous_ints_not_consumed_reports_owned_value_not_consumed() {
    let src = "
IntPair has head Int, tail Int
consume(pair IntPair) Int: 0
main:
    pair_value: IntPair(head: 1, tail: 2)
    consume(pair_value)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_owned_not_consumed = diags
        .iter()
        .any(|d| d.code.slug() == "type-owned-value-not-consumed");
    assert!(
        has_owned_not_consumed,
        "expected owned value not consumed for nominal record of anonymous ints: {:?}",
        diags
    );
}

#[test]
fn test_explicit_consumption_parameter_with_eat_handle_discharge_at_callsite() {
    let src = "
read_handle(eat handle Pointer(Int)) Int: 0
main:
    p: @42
    read_handle(eat p)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_escape = diags
        .iter()
        .any(|d| d.code.slug() == "type-consuming-parameter-escapes");
    assert!(
        !has_escape,
        "calling eat param should not leak ownership: {:?}",
        diags
    );
    let has_owned_not_consumed = diags
        .iter()
        .any(|d| d.code.slug() == "type-owned-param-not-consumed");
    assert!(
        !has_owned_not_consumed,
        "reader should not report owned-param leak: {:?}",
        diags
    );
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(
        !has_moved,
        "explicit consumption should be valid and not moved: {:?}",
        diags
    );
}

#[test]
fn test_pattern_ownership_modes_in_when_subject() {
    let src = "
Payload has value Pointer(Int)
Option(Payload) is Some(Payload) or None

observe_without_binding(value Option(Payload)) Int: when value is
    Some(_) then 0
    None then 0

observe_with_value_binding(value Option(Payload)) Int:
    when value is
        Some(v) then 0
        None then 0

observe_with_ref_binding(value Option(Payload)) Int:
    when value is
        Some(ref v) then 0
        None then 0

observe_with_mut_binding(value Option(Payload)) Int:
    when value is
        Some(mut v) then 0
        None then 0

main:
    none: None
    some: Some(Payload(value: @42))
    result: observe_without_binding(none)
    result2: observe_with_value_binding(some)
    result3: observe_with_ref_binding(Some(Payload(value: @7)))
    result4: observe_with_mut_binding(Some(Payload(value: @8)))
";
    let diags = collect_ownership_diagnostics(src);
    let bad = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(
        !bad,
        "pattern ownership modes should not emit move flaw: {:?}",
        diags
    );
}

#[test]
fn test_mut_call_preserves_existing_reference_without_invalidation_effect() {
    let src = "\
replace(mut x Int) Int:
    x: 0
return x
main:
    value: 42
    ref reference Int: value
    replace(mut value)
    result: reference
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "unexpected invalidated reference diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_fixed_array_item_replacement_invalidates_equal_reference() {
    let src = "\
main:
    items: (0; 2)
    ref saved Int: items.(0)
    items.(0):: 1
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "expected equal-place invalidation diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_fixed_array_disjoint_item_replacement_preserves_reference() {
    let src = "\
main:
    items: (0; 2)
    ref saved Int: items.(0)
    items.(1):: 1
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "proven-disjoint sibling write should preserve the reference: {diags:?}"
    );
}

#[test]
fn test_symbolic_item_replacement_conservatively_invalidates_reference() {
    let src = "\
main(i Int, j Int):
    items: (0; 2)
    ref saved Int: items.(i)
    items.(j):: 1
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "unknown-overlap write should invalidate the reference: {diags:?}"
    );
}

#[test]
fn test_ancestor_replacement_invalidates_descendant_reference() {
    let src = "\
main:
    items: (0; 2)
    ref saved Int: items.(0)
    items:: (1; 2)
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "ancestor write should invalidate a descendant borrow: {diags:?}"
    );
}

#[test]
fn test_descendant_replacement_invalidates_ancestor_reference() {
    let src = "\
Pair has head Int, tail Int
main:
    pair: Pair(head: 1, tail: 2)
    ref saved Pair: pair
    pair.head:: 3
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "descendant write should invalidate an ancestor borrow: {diags:?}"
    );
}

#[test]
fn test_consuming_different_fixed_array_item_preserves_reference() {
    let src = "\
drop(eat value Int) Int: 0
main:
    items: (0; 2)
    ref saved Int: items.(0)
    drop(eat items.(1))
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "unexpected invalidated reference diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_fixed_array_call_proves_index_refinement_from_length() {
    let src = "\
get(ref array Array(x, n), index Int and >= 0 and < n) ref x: array.(index)
main:
    items: (0; 4)
    ref saved Int: get(ref items, 2)
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags.iter().any(|d| {
            matches!(
                d.code.slug(),
                "type-parameter-refinement-failed" | "type-parameter-refinement-unproven"
            )
        }),
        "unexpected refinement diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_fixed_array_call_rejects_index_at_length() {
    let src = "\
get(ref array Array(x, n), index Int and >= 0 and < n) ref x: array.(index)
main:
    items: (0; 4)
    ref saved Int: get(ref items, 4)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-parameter-refinement-failed"),
        "expected failed refinement diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_fixed_array_call_requires_proof_for_symbolic_index() {
    let src = "\
get(ref array Array(x, n), index Int and >= 0 and < n) ref x: array.(index)
visit(ref items Array(Int, 4), index Int) Int:
    ref saved Int: get(ref items, index)
return 0
main: 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-parameter-refinement-unproven"),
        "expected unproven refinement diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_parameter_refinement_proves_symbolic_array_items_disjoint() {
    let src = "\
drop(eat value Int) Int: 0
visit(ref items x, i Int, j Int and > i) Int:
    ref saved Int: items.(i)
    drop(eat items.(j))
    result: saved
return 0
main: 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "unexpected invalidated reference diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_unrelated_symbolic_array_items_conservatively_overlap() {
    let src = "\
drop(eat value Int) Int: 0
visit(ref items x, i Int, j Int) Int:
    ref saved Int: items.(i)
    drop(eat items.(j))
    result: saved
return 0
main: 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "expected invalidated reference diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_consuming_same_fixed_array_item_invalidates_reference() {
    let src = "\
drop(eat value Int) Int: 0
main:
    items: (0; 2)
    ref saved Int: items.(0)
    drop(eat items.(0))
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "expected invalidated reference diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_eat_invalidates_existing_reference_on_later_use() {
    let src = "\
drop(eat x Int) Int: 0
main:
    value: 42
    ref reference Int: value
    drop(eat value)
    result: reference
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "expected invalidated reference diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_gate4_rebind_immutable_reports_error() {
    let src = "\
main:
    counter := 1
    counter:: 2
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-immutable"),
        "expected immutable rebind diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_gate4_rebind_nearest_mutable_succeeds() {
    let src = "\
main:
    counter: 0
    counter:: counter + 1
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_immutable = diags
        .iter()
        .any(|d| d.code.slug() == "type-rebind-immutable");
    assert!(
        !has_immutable,
        "unexpected immutable diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_gate4_rebind_non_discardable_without_transfer() {
    let src = "\
IntPair has head Int, tail Int
main:
    buffer: IntPair(head: 1, tail: 2)
    buffer:: IntPair(head: 3, tail: 4)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-would-discard-value"),
        "expected rebind discard diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_gate4_rebind_discardable_value_succeeds() {
    let src = "\
main:
    counter: 1
    counter:: 2
return counter
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-would-discard-value"),
        "discardable replacement should succeed: {diags:?}"
    );
}

#[test]
fn test_gate4_rebind_transfers_non_discardable_old_value() {
    let src = "\
IntPair has head Int, tail Int
reopen(eat old IntPair) IntPair extern
main:
    buffer: IntPair(head: 1, tail: 2)
    buffer:: reopen(eat buffer)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-would-discard-value"),
        "explicit transfer should discharge the old value: {diags:?}"
    );
}

#[test]
fn test_gate4_permanent_shadow_rejects_live_non_discardable_value() {
    let src = "\
IntPair has head Int, tail Int
main:
    buffer: IntPair(head: 1, tail: 2)
    buffer: IntPair(head: 3, tail: 4)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-shadow-would-hide-live-value"),
        "expected permanent-shadow diagnostic: {diags:?}"
    );
}

#[test]
fn test_gate4_shadow_initializer_can_transfer_old_value() {
    let src = "\
IntPair has head Int, tail Int
reopen(eat old IntPair) IntPair extern
main:
    buffer: IntPair(head: 1, tail: 2)
    buffer: reopen(eat buffer)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-shadow-would-hide-live-value"),
        "initializer transfer should discharge the hidden value: {diags:?}"
    );
}

#[test]
fn test_gate4_child_scope_can_temporarily_shadow_owned_value() {
    let src = "\
IntPair has head Int, tail Int
main(flag Int):
    buffer: IntPair(head: 1, tail: 2)
    if flag is 1
        buffer: IntPair(head: 3, tail: 4)
    return
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-shadow-would-hide-live-value"),
        "child-scope shadowing should be temporary: {diags:?}"
    );
}

#[test]
fn test_gate4_rebind_target_not_found_reports_error() {
    let src = "\
bad(value Int) ref Int extern
Small is in 0...255
main:
    missing ref Small: bad(1)
    value Small: 1
    missing:: value
return
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-target-not-found"),
        "expected target-not-found diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_gate4_rebind_crosses_callable_boundary() {
    let src = "\
next(x Int) Int: x
bad(value Int) ref Int extern
Small is in 0...255
main:
    value Small: 1
    next ref Small: bad(1)
    next:: value
return
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-crosses-callable-boundary"),
        "expected callable-boundary diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_gate4_rebind_violates_validity_reports_error() {
    let src = "\
Small is in 0...2
main:
    counter Small: 2
    counter:: 3
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-violates-validity"),
        "expected violates-validity diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_gate4_rebind_violates_validity_with_proof_succeeds() {
    let src = "\
Small2 is in 0...3
main:
    counter Small2: 0
    counter:: 2
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_violates_validity = diags
        .iter()
        .any(|d| d.code.slug() == "type-rebind-violates-validity");
    assert!(
        !has_violates_validity,
        "unexpected rebind validity diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_gate4_branch_proof_allows_bounded_increment_rebind() {
    let src = "\
Small is in 0...3
#operator(Add)
#inline
small_add(a Small, b Small) Int: (a as Int) + (b as Int)
advance(counter Small):
    if counter is < 3
        counter:: (eat counter) + 1
    return
return
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-violates-validity"),
        "branch evidence should prove the bounded increment: {diags:?}"
    );
}

#[test]
fn test_gate4_unproven_increment_rebind_violates_validity() {
    let src = "\
Small is in 0...3
#operator(Add)
#inline
small_add(a Small, b Small) Int: (a as Int) + (b as Int)
advance(counter Small):
    counter:: (eat counter) + 1
return
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-violates-validity"),
        "the same increment without branch evidence must be rejected: {diags:?}"
    );
}

#[test]
fn test_gate4_use_before_initialization_reports_error() {
    let src = "\
main:
    value Int
    result: value
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-before-initialization"),
        "expected use-before-initialization diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_gate4_all_predecessors_initialize_place() {
    let src = "\
main(flag Int):
    value Int
    when flag is
        1 then value:: 1
        else value:: 2
    observed: value
return observed
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-use-before-initialization"),
        "all predecessors initialize the place: {diags:?}"
    );
}

#[test]
fn test_gate4_one_predecessor_leaves_place_uninitialized() {
    let src = "\
main(flag Int):
    value Int
    if flag is 1
        value:: 1
    return
    observed: value
return observed
";
    let diags = collect_ownership_diagnostics(src);
    let diagnostic = diags
        .iter()
        .find(|d| d.code.slug() == "type-use-before-initialization")
        .unwrap_or_else(|| panic!("expected a maybe-uninitialized diagnostic: {diags:?}"));
    assert!(
        diagnostic
            .related
            .iter()
            .any(|related| related.label == "declared without an initializer"),
        "expected the declaration path label: {diagnostic:?}"
    );
    assert!(
        diagnostic
            .related
            .iter()
            .any(|related| related.label == "initialized on this predecessor"),
        "expected the initialized predecessor label: {diagnostic:?}"
    );
}

#[test]
fn test_gate4_consumed_place_can_be_reinitialized() {
    let src = "\
drop(eat value Pointer(Int)): 0
main:
    value: @1
    drop(eat value)
    value:: @2
    observed: deref value
return observed
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags.iter().any(|d| matches!(
            d.code.slug(),
            "type-use-after-move" | "type-use-before-initialization"
        )),
        "reinitialization should restore the place: {diags:?}"
    );
}

#[test]
fn test_gate4_ref_parameter_cannot_be_rebound() {
    let src = "\
bad(ref value Int): value:: 2
main:
    value: 1
    bad(ref value)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-immutable"),
        "expected immutable ref-parameter diagnostic: {diags:?}"
    );
}

#[test]
fn test_gate4_mut_parameter_can_write_through_borrow() {
    let src = "\
replace(mut value Int): value:: 2
main:
    value: 1
    replace(mut value)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-rebind-immutable"),
        "mut parameter should permit replacement: {diags:?}"
    );
}

mod support;

#[test]
fn test_bare_param_defaults_to_ownership() {
    let src = "print(s String):\nreturn 0\n";
    let ast = parser::cursor::TokenCursor::parse_source(src);
    let parsed = ast.defs.get(&Intern::from_ref("print")).unwrap();
    assert!(
        parsed
            .param_conventions
            .get(&Intern::from_ref("s"))
            .is_none()
    );

    let typed = transform_file(ast, FileId(0));
    let bind = typed
        .defs
        .get(&typecheck::DefId(Intern::from_ref("print")))
        .unwrap();
    assert_eq!(bind.param_conventions, vec![ast::ParamConvention::Own]);
}
