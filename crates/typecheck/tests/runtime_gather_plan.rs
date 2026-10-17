mod support;

use support::expect_flaws_with_named;

const PRELUDE: &str = concat!(
    include_str!("../../test-fixtures/fixtures/runtime_gather_plan.gin"),
    "\n"
);

const CASES: &[(&str, &str, &[&str])] = &[
    (
        "validated_indices_survive_runtime_iteration_and_helper_calls",
        "",
        &[],
    ),
    (
        "a_stored_index_must_be_strictly_below_its_extent",
        "invalid ValidatedIndex: ValidatedIndex(extent: 128, position: 128)\n",
        &["dep-refinement-failed"],
    ),
    (
        "a_missing_extent_match_cannot_authorize_the_proven_reader",
        "read(index ref ValidatedIndex, view ref View(Byte, 65535)) Byte: return proven_at(index, view)\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "a_match_for_another_view_cannot_authorize_the_proven_reader",
        "read(index ref ValidatedIndex, view ref View(Byte, 65535), other ref View(Byte, 65535)) Byte:\n    when other.length.as_num = index.extent.as_num then\n        return proven_at(index, view)\n    return 0\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "an_element_borrow_cannot_escape_its_bound",
        "borrow(index mut ValidatedIndex) mut Count: return mut index.position\n",
        &["dep-refinement-unproven"],
    ),
    (
        "mutating_an_element_must_preserve_its_bound",
        "replace_count(previous Count, next Count) Count: return next\nchange(index mut ValidatedIndex):\n    index.position :: replace_count(index.position, 65535)\n    return\n",
        &["dep-refinement-failed"],
    ),
    (
        "shrinking_an_extent_must_preserve_its_elements_bound",
        "replace_count(previous Count, next Count) Count: return next\nchange(index mut ValidatedIndex):\n    index.extent :: replace_count(index.extent, 0)\n    return\n",
        &["dep-refinement-failed"],
    ),
    (
        "changing_a_valid_extent_invalidates_a_previous_view_match",
        "replace_count(previous Count, next Count) Count: return next\nchange(index mut ValidatedIndex, view ref View(Byte, 65535)) Byte:\n    when view.length.as_num = index.extent.as_num then\n        index.extent :: replace_count(index.extent, 65535)\n        return proven_at(index, view)\n    return 0\n",
        &["type-parameter-refinement-unproven"],
    ),
];

#[test]
fn runtime_gather_plan_cases() {
    for &(name, body, expected) in CASES {
        expect_flaws_with_named(PRELUDE, body, expected, name);
    }
}
