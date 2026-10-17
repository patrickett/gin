mod support;

use support::expect_flaws_with_named;

const PRELUDE: &str = concat!(
    include_str!("../../test-fixtures/fixtures/validated_gather_plan.gin"),
    "\n"
);

const CASES: &[(&str, &str, &[&str])] = &[
    (
        "validation_constructs_a_plan_and_matching_views_can_reuse_it",
        "apply(plan ref GatherPlan, view ref View(Byte)) Byte:\n    when view.length.as_num = plan.length.as_num then\n        return read_first(plan, view)\n    return 0\n",
        &[],
    ),
    (
        "a_different_extent_cannot_use_a_plans_indices",
        "apply(plan ref GatherPlan, view ref View(Byte)) Byte: return read_first(plan, view)\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "an_invalid_index_cannot_be_stored_in_a_plan",
        "bad GatherPlan: GatherPlan(length: 4, first: 4, second: 0, third: 2, fourth: 3)\n",
        &["dep-refinement-failed"],
    ),
    (
        "shrinking_a_plan_requires_preserving_all_index_bounds",
        "shrink(plan mut GatherPlan):\n    plan.length :: Count.replace(plan.length, 0)\n    return\n",
        &["dep-refinement-failed"; 4],
    ),
    (
        "changing_an_index_requires_preserving_its_bound",
        "change(plan mut GatherPlan):\n    plan.first :: Count.replace(plan.first, 65535)\n    return\n",
        &["dep-refinement-failed"],
    ),
    (
        "checking_one_buffers_extent_does_not_authorize_another",
        "apply(plan ref GatherPlan, left ref View(Byte), right ref View(Byte)) Byte:\n    when left.length.as_num = plan.length.as_num then\n        return read_first(plan, right)\n    return 0\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "a_mutable_index_borrow_cannot_bypass_the_plan_invariant",
        "borrow(plan mut GatherPlan) mut Count: return mut plan.first\n",
        &["dep-refinement-unproven"],
    ),
    (
        "a_valid_extent_change_requires_a_new_buffer_match",
        "change(plan mut GatherPlan, view ref View(Byte)) Byte:\n    when view.length.as_num = plan.length.as_num then\n        plan.length :: Count.replace(plan.length, 65535)\n        return read_first(plan, view)\n    return 0\n",
        &["type-parameter-refinement-unproven"],
    ),
];

#[test]
fn validated_gather_plan_cases() {
    for &(name, body, expected) in CASES {
        expect_flaws_with_named(PRELUDE, body, expected, name);
    }
}
