mod support;

use support::{expect_flaws, expect_flaws_with_named, flaw_codes, package, package_files};

const REGION: &str = "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\nCount is in 0 to 255\nCount has Numeric\n    release(self):\n        return\nreplace(previous Count, next Count) Count: return next\nRegion has\n    capacity Count\n    initialized Count and <= capacity\n    initialized_count(self ref) is in Count: return self.initialized.as_num\n    capacity_count(self ref) is in Count: return self.capacity.as_num\n    empty(self ref and self.initialized_count = 0) Count: return 0\n    pop(self mut and self.initialized_count > 0) Count:\n        count: self.initialized.as_num\n        self.initialized :: replace(self.initialized, (count - 1) as Count)\n        return 0\n    clear(self mut):\n        self.initialized :: replace(self.initialized, 0)\n        return\n    release(self):\n        return\n";

const CASES: &[(&str, &str, &str, &[&str])] = &[
    (
        "bare_return_verifies_the_final_receiver_state",
        REGION,
        "clear(region mut Region):\n    region.clear\n    return thus region.initialized_count = 0\n",
        &[],
    ),
    (
        "return_postconditions_follow_mutation_through_an_alias",
        REGION,
        "clear(region mut Region):\n    alias: mut region\n    alias.clear\n    return thus region.initialized_count = 0\n",
        &[],
    ),
    (
        "a_saved_comparison_cannot_prove_a_postcondition_after_unknown_mutation",
        REGION,
        "maybe_clear(region mut Region, flag ref Count):\n    when flag.as_num > 0 then\n        region.clear\n        return\n    return\ncheck(region mut Region, flag ref Count):\n    decision: region.initialized_count > 0\n    maybe_clear(region, flag)\n    when decision then\n        return thus region.initialized_count > 0\n    return\n",
        &["type-postcondition-unproven"],
    ),
    (
        "a_saved_comparison_still_proves_its_saved_input",
        REGION,
        "maybe_clear(region mut Region, flag ref Count):\n    when flag.as_num > 0 then\n        region.clear\n        return\n    return\ncheck(region mut Region, flag ref Count):\n    before: region.initialized_count\n    decision: before > 0\n    maybe_clear(region, flag)\n    when decision then\n        return thus before > 0\n    return\n",
        &[],
    ),
    (
        "direct_scalar_mutation_establishes_the_final_value",
        "",
        "clear(value mut is in 0 to 255):\n    value :: 0\n    return thus value = 0\n",
        &[],
    ),
    (
        "a_saved_scalar_comparison_cannot_prove_the_reassigned_value",
        "",
        "clear(value mut is in 0 to 255):\n    decision: value > 0\n    value :: 0\n    when decision then\n        return thus value > 0\n    return\n",
        &["type-postcondition-failed"],
    ),
    (
        "a_taken_conjunction_proves_its_cleanup_ran",
        REGION,
        "clear_true(region mut Region):\n    region.clear\n    return True\ncheck(region mut Region, flag ref Count):\n    when flag.as_num > 0 and clear_true(region) is True then\n        return thus region.initialized_count = 0\n    return\n",
        &[],
    ),
    (
        "a_short_circuited_mutation_cannot_prove_cleanup",
        REGION,
        "clear_true(region mut Region):\n    region.clear\n    return True\ncheck(region mut Region, flag ref Count):\n    when flag.as_num > 0 and clear_true(region) is True then\n        return\n    return thus region.initialized_count = 0\n",
        &["type-postcondition-unproven"],
    ),
    (
        "a_return_cannot_claim_the_incoming_receiver_state",
        REGION,
        "clear(region mut Region and region.initialized_count > 0):\n    region.clear\n    return thus region.initialized_count > 0\n",
        &["type-postcondition-failed"],
    ),
    (
        "a_return_requires_proof",
        REGION,
        "read(region ref Region):\n    return thus region.initialized_count = 0\n",
        &["type-postcondition-unproven"],
    ),
    (
        "return_expressions_run_before_the_postcondition_is_checked",
        REGION,
        "take(region mut Region and region.initialized_count = 1) Count:\n    return region.pop thus region.initialized_count = 0\n",
        &[],
    ),
    (
        "a_postcondition_can_use_a_saved_incoming_value",
        REGION,
        "take(region mut Region and region.initialized_count > 0) Count:\n    before: region.initialized_count\n    value: region.pop\n    return value thus region.initialized_count = before - 1\n",
        &[],
    ),
    (
        "early_return_checks_its_branch_state",
        REGION,
        "clear(region mut Region):\n    when region.initialized_count > 0 then\n        region.clear\n        return thus region.initialized_count = 0\n    return thus region.initialized_count = 0\n",
        &[],
    ),
    (
        "an_early_return_cannot_claim_cleanup_without_performing_it",
        REGION,
        "clear(region mut Region):\n    when region.initialized_count > 0 then\n        return thus region.initialized_count = 0\n    return thus region.initialized_count = 0\n",
        &["type-postcondition-failed"],
    ),
    (
        "return_postconditions_can_refer_to_compile_time_constants",
        "",
        "limit := 7\ncheck(value is in 0 to 255 and value = limit):\n    return thus value = limit\n",
        &[],
    ),
    (
        "cleanup_loop_proves_zero_on_normal_exit",
        REGION,
        "clear(region mut Region):\n    while region.initialized_count > 0 do\n        value: region.pop\n        value.release\n    return thus region.initialized_count = 0\n",
        &[],
    ),
    (
        "cleanup_loop_guarantee_is_available_to_the_caller",
        REGION,
        "clear(region mut Region):\n    while region.initialized_count > 0 do\n        value: region.pop\n        value.release\n    return thus region.initialized_count = 0\nrun:\n    region: Region(4, 2)\n    clear(region)\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "an_unannotated_early_exit_cannot_publish_the_final_exits_guarantee",
        REGION,
        "clear(region mut Region):\n    when region.initialized_count > 0 then\n        return\n    return thus region.initialized_count = 0\nrun:\n    region: Region(4, 2)\n    clear(region)\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "boolean_returns_verify_complementary_postconditions",
        "",
        "less(left is in 0 to 255, right is in 0 to 255):\n    when left < right then\n        return True thus left < right\n    return False thus left >= right\n",
        &[],
    ),
    (
        "inferred_boolean_contracts_allow_the_caller_to_narrow",
        "",
        "Byte is in 0 to 255\nByte has release(self): return\nless(left ref is in 0 to 511, right is in 0 to 511):\n    when left < right then\n        return True thus left < right\n    return False thus left >= right\nnarrow(value is in 0 to 511) Byte:\n    when less(value, 256) is True then\n        return value as Byte\n    return 0\n",
        &[],
    ),
    (
        "a_loop_cannot_claim_more_than_its_exit_condition",
        REGION,
        "clear(region mut Region):\n    while region.initialized_count > 1 do\n        value: region.pop\n        value.release\n    return thus region.initialized_count = 0\n",
        &["type-postcondition-unproven"],
    ),
    (
        "a_loop_does_not_reuse_a_concrete_incoming_count",
        REGION,
        "clear(region mut Region and region.initialized_count = 2):\n    while region.initialized_count > 0 do\n        value: region.pop\n        value.release\n    return thus region.initialized_count = 2\n",
        &["type-postcondition-failed"],
    ),
    (
        "cleanup_preserves_capacity_and_a_saved_count",
        REGION,
        "clear(region mut Region):\n    capacity: region.capacity_count\n    before: region.initialized_count\n    while region.initialized_count > 0 do\n        value: region.pop\n        value.release\n    return thus region.initialized_count = 0 and region.capacity_count = capacity and before >= 0\n",
        &[],
    ),
    (
        "an_early_loop_exit_must_prove_its_own_assertion",
        REGION,
        "clear(region mut Region):\n    while region.initialized_count > 0 do\n        when region.initialized_count = 1 then\n            return thus region.initialized_count = 0\n        value: region.pop\n        value.release\n    return thus region.initialized_count = 0\n",
        &["type-postcondition-failed"],
    ),
    (
        "a_distinct_returned_local_does_not_publish_a_guarantee_about_the_parameter",
        REGION,
        "clear_local(region mut Region) Region:\n    local: Region(4, 1)\n    local.clear\n    return local thus local.initialized = 0\nrun:\n    region: Region(4, 1)\n    result: clear_local(region)\n    result.release\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
];

#[test]
fn return_postcondition_cases() {
    for &(name, prelude, body, expected) in CASES {
        expect_flaws_with_named(prelude, body, expected, name);
    }
}

#[test]
fn postcondition_operands_must_be_numeric_and_known() {
    expect_flaws(
        &format!("{REGION}read(region ref Region):\n    return thus missing = 0\n"),
        &["type-constraint-unknown-name"],
    );
    expect_flaws(
        &format!("{REGION}read(region ref Region):\n    return thus region = 0\n"),
        &["type-constraint-noninteger"],
    );
    expect_flaws(
        "A is in 0 to 255\nA has release(self): return\nB is in 0 to 255\nB has release(self): return\ncompare(left ref A, right ref B):\n    return thus left = right\n",
        &["type-constraint-type-mismatch"],
    );
}

const CLEANUP: &str = "clear(region mut Region):\n    while region.initialized_count > 0 do\n        value: region.pop\n        value.release\n    return thus region.initialized_count = 0\n";
const CLIENT: &str = "use sample.model Count, Region, clear\nrun:\n    region: Region(capacity: 4, initialized: 2)\n    clear(region)\n    value: region.empty\n    value.release\n    region.release\n    return\n";

#[test]
fn cleanup_postconditions_cross_module_boundaries() {
    let model = format!("{REGION}{CLEANUP}");
    for reversed in [false, true] {
        let mut files = [("model", model.as_str()), ("client", CLIENT)];
        if reversed {
            files.reverse();
        }
        let output = package("sample", &files);
        let flaws: Vec<_> = output.iter().flat_map(|file| file.all_flaws()).collect();
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}

#[test]
fn removing_a_postcondition_invalidates_a_cached_callers_proof() {
    use typecheck::transform::{
        BodyReuse, PackageCheckSession, PackageTransformOptions,
        transform_package_with_shared_context_and_session,
    };
    let mut session =
        PackageCheckSession::new(1.try_into().unwrap(), BodyReuse::ExactInputs).unwrap();
    let mut fingerprints = Vec::new();
    for annotated in [true, true, false, false, true] {
        let model = format!(
            "{REGION}{}",
            if annotated {
                CLEANUP.to_owned()
            } else {
                CLEANUP.replace(" thus region.initialized_count = 0", "")
            }
        );
        let output = transform_package_with_shared_context_and_session(
            package_files("sample", &[("model", &model), ("client", CLIENT)]),
            PackageTransformOptions::FULL,
            ast::ty::PackageInstanceKey::workspace("sample", "1"),
            &mut session,
        );
        let codes: Vec<_> = output.typed_asts.iter().flat_map(flaw_codes).collect();
        assert_eq!(
            codes,
            if annotated {
                vec![]
            } else {
                vec!["type-parameter-refinement-unproven"]
            }
        );
        fingerprints.push(output.public_interface.semantic_surface_fingerprint);
    }
    assert_eq!(fingerprints[0], fingerprints[1]);
    assert_ne!(fingerprints[1], fingerprints[2]);
    assert_eq!(fingerprints[2], fingerprints[3]);
    assert_eq!(fingerprints[0], fingerprints[4]);
}
