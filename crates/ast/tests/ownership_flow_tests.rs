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
//! NOTE: standard-library modules are NOT loaded in these tests. User-defined types
//! resolve to Ty::Opaque (8 bytes, ≤ 16, Copy) when they can't be found.
//! LinValueNotConsumed flow tests are therefore limited — the copyability unit
//! tests in `test_copy_inference_*` directly verify the size-based rule.

use internment::Intern;
use typecheck::FileId;
use typecheck::transform::transform_file;

/// `Int` tag so ownership tests can type parameters without loading a library.
const INT_TAG: &str = "Int is in 0 to 400\nInt has\n    finish(self):\n        return\n\n";

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

fn assert_has_slug(diags: &[diagnostic::Diagnostic], slug: &str) {
    assert!(
        diags.iter().any(|d| d.code.slug() == slug),
        "expected `{slug}` diagnostic, got: {diags:?}"
    );
}

fn assert_lacks_slug(diags: &[diagnostic::Diagnostic], slug: &str) {
    assert!(
        !diags.iter().any(|d| d.code.slug() == slug),
        "unexpected `{slug}` diagnostic: {diags:?}"
    );
}

/// Uniform has/lacks-slug cases; the first tuple field is the former test name,
/// kept so failures identify the case.
#[rustfmt::skip]
const OWNERSHIP_CASES: &[(&str, &str, &str, bool)] = &[
    // Consume via `eat` at call site — subsequent use should produce UseOfMovedValue.
    ("test_use_after_move_produces_diagnostic", "drop(x Int) Int: return 0\nmain:\n    val: 42\n    drop(val)\n    result: val\n    return 0\n", "type-use-of-moved-value", true),
    // Two consecutive `eat` on the same value — second is UseOfMovedValue.
    ("test_double_consume_produces_diagnostic", "drop(x Int) Int: return 0\nmain:\n    val: 42\n    drop(val)\n    drop(val)\n    return 0\n", "type-use-of-moved-value", true),
    // Copyable (Int) value used normally — no UseOfMovedValue.
    ("test_no_false_positive_use_after_move_copyable", "\
main:
    val: 42
    result: val + 1
    return 0
", "type-use-of-moved-value", false),
    // Each ownership pass copies a structurally Copy value.
    ("test_default_owned_copy_arguments_can_be_chained", "add_one(x Int) Int: return x + 1\ndouble(y Int) Int: return y * 2\nmain:\n    val: 42\n    s1: add_one(val)\n    r: double(s1)\n    return r\n", "type-use-of-moved-value", false),
    ("test_move_on_one_when_path_is_maybe_moved_after_join", "sink(pointer @Int) Int: return pointer\nselect(flag Int, pointer @Int) Int:\n    when flag is\n        0 then sink(pointer)\n        else 0\n    result: deref pointer\n    return result\n", "type-use-of-maybe-moved-value", true),
    ("test_loop_backedge_rejects_reusing_moved_value", "sink(pointer @Int): return pointer\nrepeat(pointer @Int):\n    while 1 is 1 do\n        sink(pointer)\n    return\n", "type-use-of-moved-value", true),
    ("test_observe_then_consume", "read(x ref Int) Int: return x\ndrop(y Int) Int: return 0\nmain:\n    val: 42\n    read(val)\n    drop(val)\n    return 0\n", "type-use-of-moved-value", false),
    ("test_partial_record_destructure_discards_non_discardable_remainder", "\nPoint has\n    head Int, tail @Int\n    finish(self):\n        return\nmain:\n    value: 42\n    point := Point(head: value, tail: @value)\n    Point(head: projected) := point\n", "type-pattern-would-discard-value", true),
    ("test_partial_record_destructure_with_nominal_remainder_reports_discard_error", "\nPoint has\n    head Int, tail @Int\n    finish(self):\n        return\nmain:\n    ptr: @42\n    point: Point(head: 1, tail: ptr)\n    Point(head: value) := point\n", "type-pattern-would-discard-value", true),
    ("test_discardable_sum_with_nullary_alternative_is_allowed", "
Option(Int) is Some(Int) or None
main:
    val: None
    when val is
        Some(_) then 0
        None then 0
", "type-pattern-would-discard-value", false),
    ("test_sum_discardability_with_reachable_nominal_payload_and_proven_branch_is_allowed", "
ValueCell has value @Int
CellMaybe is Some(ValueCell) or None
main:
    cell: @42
    val: Some(ValueCell(value: cell))
    when val is
        Some(_) then 0
", "type-pattern-would-discard-value", false),
    ("test_default_owned_argument_is_valid_without_callsite_marker", "read(x Int) Int: return x\nmain:\n    val: 42\n    read(val)\n    return 0\n", "type-argument-mode-mismatch", false),
    // `eat` param that is never used in the body — auto-dropped, no diagnostic.
    ("test_eat_param_unused_is_valid", "leaky(x Int) Int: return 0\nmain:\n    return 0\n", "type-lin-value-not-consumed", false),
    ("test_pattern_ownership_modes_in_when_subject", "\nPayload has value @Int\nOption(Payload) is Some(Payload) or None\n\nobserve_without_binding(value Option(Payload)) Int: return when value is\n    Some(_) then 0\n    None then 0\n\nobserve_with_value_binding(value Option(Payload)) Int:\n    when value is\n        Some(v) then 0\n        None then 0\n\nobserve_with_ref_binding(value Option(Payload)) Int:\n    when value is\n        Some(ref v) then 0\n        None then 0\n\nobserve_with_mut_binding(value Option(Payload)) Int:\n    when value is\n        Some(mut v) then 0\n        None then 0\n\nmain:\n    none: None\n    some: Some(Payload(value: @42))\n    result: observe_without_binding(none)\n    result2: observe_with_value_binding(some)\n    result3: observe_with_ref_binding(Some(Payload(value: @7)))\n    result4: observe_with_mut_binding(Some(Payload(value: @8)))\n", "type-use-of-moved-value", false),
    ("test_mut_call_preserves_existing_reference_without_invalidation_effect", "\
replace(x mut Int) Int:
    local Int: 1
    return local
main:
    value Int: 42
    reference ref Int: value
    replace(value)
    result: reference
    return 0
", "type-use-of-invalidated-reference", false),
    ("test_fixed_array_item_replacement_invalidates_equal_reference", "\
main:
    items: [0; 2]
    ref saved Int: items.[0]
    items.[0]:: 1
    result: saved
    return 0
", "type-use-of-invalidated-reference", true),
    ("test_fixed_array_disjoint_item_replacement_preserves_reference", "\
main:
    items: [0; 2]
    ref saved Int: items.[0]
    items.[1]:: 1
    result: saved
    return 0
", "type-use-of-invalidated-reference", false),
    ("test_symbolic_item_replacement_conservatively_invalidates_reference", "\
main(i Int, j Int):
    items: [0; 2]
    ref saved Int: items.[i]
    items.[j]:: 1
    result: saved
    return 0
", "type-use-of-invalidated-reference", true),
    ("test_ancestor_replacement_invalidates_descendant_reference", "\
main:
    items: [0; 2]
    ref saved Int: items.[0]
    items:: [1; 2]
    result: saved
    return 0
", "type-use-of-invalidated-reference", true),
    ("test_descendant_replacement_invalidates_ancestor_reference", "\
Pair has head Int, tail Int
main:
    pair: Pair(head: 1, tail: 2)
    ref saved Pair: pair
    pair.head:: 3
    result: saved
    return 0
", "type-use-of-invalidated-reference", true),
    ("test_consuming_different_fixed_array_item_preserves_reference", "drop(value Int) Int: return 0\nmain:\n    items: [0; 2]\n    saved ref Int: items.[0]\n    drop(items.[1])\n    result: saved\n    return 0\n", "type-use-of-invalidated-reference", false),
    ("test_fixed_array_call_rejects_index_at_length", "get(array ref [Int; 4], index Int and >= 0 and < 4) ref Int: return array.[index]\nmain:\n    items: [0; 4]\n    saved ref Int: get(items, 4)\n    return 0\n", "type-parameter-refinement-failed", true),
    ("test_fixed_array_call_requires_proof_for_symbolic_index", "get(array ref [Int; 4], index Int and >= 0 and < 4) ref Int: return array.[index]\nvisit(items ref [Int; 4], index Int) Int:\n    saved ref Int: get(items, index)\n    return 0\nmain: 0\n", "type-parameter-refinement-unproven", true),
    ("test_parameter_refinement_proves_symbolic_array_items_disjoint", "drop(value Int) Int: return 0\nvisit(items ref [Int; 4], i is in 0 to 3, j is in 0 to 3 and > i) Int:\n    saved ref Int: items.[i]\n    drop(items.[j])\n    result: saved\n    return 0\nmain: 0\n", "type-use-of-invalidated-reference", false),
    ("test_unrelated_symbolic_array_items_conservatively_overlap", "drop(value Int) Int: return 0\nvisit(items ref [Int; 4], i is in 0 to 3, j is in 0 to 3) Int:\n    saved ref Int: items.[i]\n    drop(items.[j])\n    result: saved\n    return 0\nmain: 0\n", "type-use-of-invalidated-reference", true),
    ("test_consuming_same_fixed_array_item_invalidates_reference", "drop(value Int) Int: return 0\nmain:\n    items: [0; 2]\n    saved ref Int: items.[0]\n    drop(items.[0])\n    result: saved\n    return 0\n", "type-use-of-invalidated-reference", true),
    ("test_eat_invalidates_existing_reference_on_later_use", "drop(x Int) Int: return 0\nmain:\n    value: 42\n    reference ref Int: value\n    drop(value)\n    result: reference\n    return 0\n", "type-use-of-invalidated-reference", true),
    ("test_gate4_rebind_immutable_reports_error", "\
main:
    counter := 1
    counter:: 2
    return 0
", "type-rebind-immutable", true),
    ("test_gate4_rebind_nearest_mutable_succeeds", "\
main:
    counter: 0
    counter:: counter + 1
    return 0
", "type-rebind-immutable", false),
    ("test_gate4_rebind_non_discardable_without_transfer", "IntPair has\n    head Int, tail Int\n    finish(self):\n        return\nmain:\n    buffer: IntPair(head: 1, tail: 2)\n    buffer:: IntPair(head: 3, tail: 4)\n    return 0\n", "type-rebind-would-discard-value", true),
    ("test_gate4_rebind_discardable_value_succeeds", "\
main:
    counter: 1
    counter:: 2
    return counter
", "type-rebind-would-discard-value", false),
    ("test_gate4_rebind_transfers_non_discardable_old_value", r#"IntPair has
    head Int, tail Int
    finish(self):
        return
reopen(old IntPair) IntPair: return old
main:
    buffer: IntPair(head: 1, tail: 2)
    buffer:: reopen(buffer)
    return 0
"#, "type-rebind-would-discard-value", false),
    ("test_gate4_fresh_binding_rejects_a_visible_owned_value", "IntPair has\n    head Int, tail Int\n    finish(self):\n        return\nmain:\n    buffer: IntPair(head: 1, tail: 2)\n    buffer: IntPair(head: 3, tail: 4)\n    return 0\n", "type-binding-shadows-visible-name", true),
    ("test_gate4_fresh_binding_rejects_a_name_even_after_initializer_transfer", r#"IntPair has
    head Int, tail Int
    finish(self):
        return
reopen(old IntPair) IntPair: return old
main:
    buffer: IntPair(head: 1, tail: 2)
    buffer: reopen(buffer)
    return 0
"#, "type-binding-shadows-visible-name", true),
    ("test_gate4_child_scope_rejects_a_visible_owned_value", "IntPair has\n    head Int, tail Int\n    finish(self):\n        return\nmain(flag Int):\n    buffer: IntPair(head: 1, tail: 2)\n    when flag is 1 then\n        buffer: IntPair(head: 3, tail: 4)\n        return\n    return 0\n", "type-binding-shadows-visible-name", true),
    ("test_gate4_rebind_target_not_found_reports_error", r#"bad(value Int) ref Int: return 0
Small is in 0 to 255
main:
    missing ref Small: bad(1)
    value Small: 1
    missing:: value
    return
"#, "type-rebind-target-not-found", true),
    ("test_gate4_rebind_crosses_callable_boundary", r#"next(x Int) Int: return x
bad(value Int) ref Int: return 0
Small is in 0 to 255
main:
    value Small: 1
    next ref Small: bad(1)
    next:: value
    return
"#, "type-rebind-crosses-callable-boundary", true),
    ("test_gate4_rebind_violates_validity_reports_error", "\
Small is in 0 to 2
main:
    counter Small: 2
    counter:: 3
    return 0
", "type-rebind-violates-validity", true),
    ("test_gate4_rebind_violates_validity_with_proof_succeeds", "\
Small2 is in 0 to 3
main:
    counter Small2: 0
    counter:: 2
    return 0
", "type-rebind-violates-validity", false),
    ("test_gate4_branch_proof_allows_bounded_increment_rebind", "Small is in 0 to 3\n#infix('+')\n#inline\nsmall_add(a Small, b Small) Int: return (a as Int) + (b as Int)\nadvance(counter Small):\n    when counter is < 3 then\n        counter:: (counter) + 1\n        return\n    return\n", "type-rebind-violates-validity", false),
    ("test_gate4_unproven_increment_rebind_violates_validity", "Small is in 0 to 3\n#infix('+')\n#inline\nsmall_add(a Small, b Small) Int: return (a as Int) + (b as Int)\nadvance(counter Small):\n    counter:: (counter) + 1\n    return\n", "type-rebind-violates-validity", true),
    ("test_gate4_use_before_initialization_reports_error", "\
main:
    value Int
    result: value
    return 0
", "type-use-before-initialization", true),
    ("test_gate4_all_predecessors_initialize_place", "\
main(flag Int):
    value Int
    when flag is
        1 then value:: 1
        else value:: 2
    observed: value
    return observed
", "type-use-before-initialization", false),
    ("test_gate4_ref_parameter_cannot_be_rebound", "bad(value ref Int): return value:: 2\nmain:\n    value: 1\n    bad(value)\n    return 0\n", "type-rebind-immutable", true),
    ("test_gate4_mut_parameter_can_write_through_borrow", "replace(value mut Int): return value:: 2\nmain:\n    value: 1\n    replace(value)\n    return 0\n", "type-rebind-immutable", false),
];

#[test]
fn ownership_flow_slugs_match_expectations() {
    for (test_name, src, slug, expect_has) in OWNERSHIP_CASES {
        let diags = collect_ownership_diagnostics(src);
        let has_slug = diags.iter().any(|d| d.code.slug() == *slug);
        assert_eq!(
            has_slug, *expect_has,
            "{test_name}: expected `{slug}` present == {expect_has}; diagnostics: {diags:?}"
        );
    }
}

#[test]
fn test_move_in_when_arm_does_not_poison_sibling_arm() {
    let src = "sink(pointer @Int) Int: return pointer\nselect(flag Int, pointer @Int) Int:\n    when flag is\n        0 then sink(pointer)\n        else deref pointer\n";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        !diags.iter().any(|diagnostic| matches!(
            diagnostic.code.slug().as_str(),
            "type-use-of-moved-value" | "type-use-of-maybe-moved-value"
        )),
        "exclusive sibling arm saw moved value: {:?}",
        diags
    );
}

#[test]
fn test_reference_invalidated_on_one_when_path_is_maybe_invalid_after_join() {
    let src = "Cell has value Int\nsink(cell Cell): return cell\ncheck(flag Int, cell Cell) Int:\n    saved ref Cell: cell\n    when flag is\n        0 then sink(cell)\n        else saved.value\n    result: saved.value\n    return result\n";
    let diags = collect_ownership_diagnostics(src);

    assert_has_slug(&diags, "type-use-of-maybe-invalidated-reference");
    assert_lacks_slug(&diags, "type-use-of-invalidated-reference");
}

#[test]
fn test_nominal_record_owned_param_not_consumed_reports_owned_value_not_consumed() {
    let src = "\nPoint has\n    head Int, tail @Int\n    finish(self):\n        return\nconsume(point own Point) Int: return 0\nmain:\n    value: 42\n    point: Point(head: value, tail: @value)\n    consume(point)\n    return 0\n";
    let diags = collect_ownership_diagnostics(src);
    let has_owned_not_consumed = diags.iter().any(|d| {
        matches!(
            d.code.slug().as_str(),
            "type-owned-value-not-consumed" | "type-owned-param-not-consumed"
        )
    });
    assert!(
        has_owned_not_consumed,
        "expected owned value not consumed at callable exit: {:?}",
        diags
    );
}

#[test]
fn test_nominal_record_with_only_anonymous_ints_not_consumed_reports_owned_value_not_consumed() {
    let src = "\nIntPair has\n    head Int, tail Int\n    finish(self):\n        return\nconsume(pair own IntPair) Int: return 0\nmain:\n    pair_value: IntPair(head: 1, tail: 2)\n    consume(pair_value)\n    return 0\n";
    let diags = collect_ownership_diagnostics(src);
    let has_owned_not_consumed = diags.iter().any(|d| {
        matches!(
            d.code.slug().as_str(),
            "type-owned-value-not-consumed" | "type-owned-param-not-consumed"
        )
    });
    assert!(
        has_owned_not_consumed,
        "expected owned value not consumed for nominal record of anonymous ints: {:?}",
        diags
    );
}

#[test]
fn test_explicit_consumption_parameter_with_eat_handle_discharge_at_callsite() {
    let src = "\nread_handle(handle @Int) Int: return 0\nmain:\n    p: @42\n    read_handle(p)\n    return 0\n";
    let diags = collect_ownership_diagnostics(src);
    assert_lacks_slug(&diags, "type-argument-mode-mismatch");
    assert_lacks_slug(&diags, "type-owned-param-not-consumed");
    assert_lacks_slug(&diags, "type-use-of-moved-value");
}

#[test]
fn test_fixed_array_call_proves_index_refinement_from_length() {
    let src = "get(array ref [Int; 4], index Int and >= 0 and < 4) ref Int: return array.[index]\nmain:\n    items: [0; 4]\n    saved ref Int: get(items, 2)\n    result: saved\n    return 0\n";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags.iter().any(|d| {
            matches!(
                d.code.slug().as_str(),
                "type-parameter-refinement-failed" | "type-parameter-refinement-unproven"
            )
        }),
        "unexpected refinement diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_gate4_one_predecessor_leaves_place_uninitialized() {
    let src = "main(flag Int):\n    value Int\n    when flag is 1 then\n        value:: 1\n        return\n    observed: value\n    return observed\n";
    let diags = collect_ownership_diagnostics(src);
    let diagnostic = diags
        .iter()
        .find(|d| d.code.slug() == "type-use-before-initialization")
        .unwrap_or_else(|| panic!("expected a maybe-uninitialized diagnostic: {diags:?}"));
    assert!(
        diagnostic
            .related()
            .iter()
            .any(|related| related.label == "declared without an initializer"),
        "expected the declaration path label: {diagnostic:?}"
    );
    assert!(
        diagnostic
            .related()
            .iter()
            .any(|related| related.label == "initialized on this predecessor"),
        "expected the initialized predecessor label: {diagnostic:?}"
    );
}

#[test]
fn test_gate4_consumed_place_can_be_reinitialized() {
    let src = "drop(value @Int): return 0\nmain:\n    value: @1\n    drop(value)\n    value:: @2\n    observed: deref value\n    return observed\n";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags.iter().any(|d| matches!(
            d.code.slug().as_str(),
            "type-use-after-move" | "type-use-before-initialization"
        )),
        "reinitialization should restore the place: {diags:?}"
    );
}

mod support;

#[test]
fn test_bare_param_defaults_to_terminal() {
    let src = "print(s String):\n    return 0\n";
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
        .get(&typecheck::DefId::named(Intern::from_ref("print")))
        .unwrap();
    assert_eq!(
        bind.param_conventions.as_slice(),
        [ast::ParamConvention::Terminal].as_slice()
    );
}
