mod support;

use support::{expect_flaws_with, expect_flaws_with_named, package};

const TYPES: &str = "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\nCount is in 0 to 255\nCount has Numeric\n    release(self):\n        return\nreplace(previous Count, next Count) Count: return next\nRegion has\n    capacity Count\n    initialized Count and <= capacity\n    release(self):\n        return\n";

const CASES: &[(&str, &str, &[&str])] = &[
    (
        "initialization_cannot_exceed_capacity",
        "run:\n    region: Region(4, 5)\n    region.release\n    return\n",
        &["dep-refinement-failed"],
    ),
    (
        "a_known_capacity_proves_a_literal_replacement",
        "run:\n    region: Region(4, 0)\n    region.initialized :: replace(region.initialized, 3)\n    region.release\n    return\n",
        &[],
    ),
    (
        "reordered_constructor_arguments_preserve_field_identity",
        "run:\n    region: Region(initialized: 0, capacity: 4)\n    region.initialized :: replace(region.initialized, 3)\n    region.release\n    return\n",
        &[],
    ),
    (
        "replacing_a_constrained_field_requires_proof",
        "set(region mut Region, value Count):\n    region.initialized :: replace(region.initialized, value)\n    return\n",
        &["dep-refinement-unproven"],
    ),
    (
        "replacing_a_dependency_requires_preserving_its_dependents",
        "shrink(region mut Region):\n    region.capacity :: replace(region.capacity, 0)\n    return\n",
        &["dep-refinement-unproven"],
    ),
    (
        "an_implicit_mutable_argument_cannot_bypass_the_parent_constraint",
        "write(value mut Count, next Count) Count: return next\nset(region mut Region, next Count):\n    result: write(region.initialized, next)\n    result.release\n    return\n",
        &["dep-refinement-unproven"],
    ),
    (
        "an_unrelated_field_can_still_be_borrowed_mutably",
        "Other has\n    capacity Count\n    initialized Count and <= capacity\n    extra Count\n    release(self):\n        return\nborrow(region mut Other) mut Count: return mut region.extra\n",
        &[],
    ),
    (
        "clearing_preserves_the_invariant",
        "clear(region mut Region):\n    region.initialized :: replace(region.initialized, 0)\n    return\n",
        &[],
    ),
    (
        "a_checked_replacement_is_accepted",
        "set(region mut Region, value Count):\n    when value.as_num > region.capacity.as_num then\n        return\n    region.initialized :: replace(region.initialized, value)\n    return\n",
        &[],
    ),
    (
        "a_checked_dependency_replacement_is_accepted",
        "shrink(region mut Region, value Count):\n    when value.as_num < region.initialized.as_num then\n        return\n    region.capacity :: replace(region.capacity, value)\n    return\n",
        &[],
    ),
    (
        "an_unrelated_replacement_preserves_the_invariant",
        "Other has\n    capacity Count\n    initialized Count and <= capacity\n    extra Count\n    release(self):\n        return\nset(region mut Other, value Count):\n    region.extra :: replace(region.extra, value)\n    return\n",
        &[],
    ),
    (
        "unknown_constructor_values_are_not_accepted_as_proof",
        "read(value ref Count) Count\nrun(capacity ref Count) Region: return Region(capacity.as_num as Count, read(capacity))\n",
        &["dep-refinement-unproven"],
    ),
    (
        "clearing_allows_capacity_to_shrink_to_zero",
        "shrink(region mut Region):\n    region.initialized :: replace(region.initialized, 0)\n    region.capacity :: replace(region.capacity, 0)\n    return\n",
        &[],
    ),
    (
        "mutation_invalidates_an_earlier_capacity_check",
        "set(region mut Region, value Count):\n    when value.as_num > region.capacity.as_num then\n        return\n    region.initialized :: replace(region.initialized, 0)\n    region.capacity :: replace(region.capacity, 0)\n    region.initialized :: replace(region.initialized, value)\n    return\n",
        &["dep-refinement-unproven"],
    ),
    (
        "advancing_an_initialized_prefix_requires_available_capacity",
        "advance(region mut Region):\n    count: region.initialized.as_num\n    when count >= region.capacity.as_num then\n        return\n    region.initialized :: replace(region.initialized, (count + 1) as Count)\n    return\n",
        &[],
    ),
    (
        "changing_a_checked_scalar_invalidates_its_proof",
        "set(region mut Region, value Count):\n    amount: value\n    when amount.as_num > region.capacity.as_num then\n        return\n    amount :: replace(amount, 255)\n    region.initialized :: replace(region.initialized, amount)\n    return\n",
        &["dep-refinement-unproven"],
    ),
    (
        "a_loop_does_not_restore_old_field_values",
        "set(region mut Region, value Count):\n    region.initialized :: replace(region.initialized, 0)\n    region.capacity :: replace(region.capacity, 255)\n    while region.capacity.as_num > 0 do\n        region.initialized :: replace(region.initialized, 0)\n        region.capacity :: replace(region.capacity, 0)\n    region.initialized :: replace(region.initialized, value)\n    return\n",
        &["dep-refinement-unproven"],
    ),
    (
        "getter_proofs_refer_to_the_correct_receiver",
        "Region has\n    limit(self ref) is in Count: return self.capacity.as_num\nset(first mut Region, second ref Region, value Count):\n    when value.as_num > second.limit then\n        return\n    first.initialized :: replace(first.initialized, value)\n    return\n",
        &["dep-refinement-unproven"],
    ),
    (
        "borrowing_an_unrelated_field_preserves_arithmetic_constraints",
        "Other has\n    capacity Count and capacity * #size(Count) <= 255\n    extra Count\n    release(self): return\nborrow(region mut Other) mut Count: return mut region.extra\n",
        &[],
    ),
];

#[test]
fn record_constraint_cases() {
    for &(name, body, expected) in CASES {
        expect_flaws_with_named(TYPES, body, expected, name);
    }
}

#[test]
fn a_mutable_field_reference_cannot_bypass_the_parent_constraint() {
    expect_flaws_with(
        TYPES,
        "borrow(region mut Region) mut Count: return mut region.initialized\n",
        &["dep-refinement-unproven"],
    );
    expect_flaws_with(
        TYPES,
        "borrow(region mut Region) mut Count: return mut region.capacity\n",
        &["dep-refinement-unproven"],
    );
}

#[test]
fn constraints_and_scalar_proofs_work_across_modules() {
    for (body, expected) in [
        (
            "region.initialized :: replace(region.initialized, value)",
            true,
        ),
        (
            "when value.as_num > region.capacity.as_num then\n        return\n    region.initialized :: replace(region.initialized, value)",
            false,
        ),
    ] {
        for reversed in [false, true] {
            let client = format!(
                "use sample.model Count, Region, replace\nset(region mut Region, value Count):\n    {body}\n    return\n"
            );
            let mut files = [("model", TYPES), ("client", client.as_str())];
            if reversed {
                files.reverse();
            }
            let output = package("sample", &files);
            let flaws: Vec<_> = output.iter().flat_map(|file| file.all_flaws()).collect();
            assert_eq!(
                flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "dep-refinement-unproven"),
                expected,
                "{flaws:?}"
            );
            assert!(
                flaws
                    .iter()
                    .all(|(_, flaw)| flaw.code.slug() == "dep-refinement-unproven"),
                "{flaws:?}"
            );
        }
    }
}
