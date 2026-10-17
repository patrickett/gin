mod support;

use parser::query::SourceParseExt;
use support::{check, expect_flaws_with, expect_flaws_with_named, flaw_codes, package};

const TYPES: &str = "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\nCount is in 0 to 255\nCount has Numeric\n    release(self):\n        return\nreplace(previous Count, next Count) Count: return next\nRegion has\n    capacity Count\n    initialized Count and <= capacity\n    initialized_count(self ref) is in Count: return self.initialized.as_num\n    last(self ref and self.initialized_count > 0) Count: return 0\n    at(self ref, index Count and < self.initialized_count) Count: return index\n    release(self):\n        return\n";

const CASES: &[(&str, &str, &[&str])] = &[
    (
        "empty_receiver_fails_its_precondition",
        "run:\n    region: Region(4, 0)\n    value: region.last\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "nonempty_receiver_proves_its_precondition",
        "run:\n    region: Region(4, 2)\n    value: region.last\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "unknown_receiver_requires_a_proof",
        "read(region ref Region) Count: return region.last\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "a_guard_authorizes_the_call",
        "read(region ref Region) Count:\n    when region.initialized_count > 0 then\n        return region.last\n    return 0\n",
        &[],
    ),
    (
        "receiver_precondition_is_available_in_the_body",
        "read(region ref Region and region.initialized_count > 0) Count: return region.last\n",
        &[],
    ),
    (
        "bounds_can_depend_on_a_receiver_query",
        "read(region ref Region, index Count) Count:\n    when index.as_num < region.initialized_count then\n        return region.at(index)\n    return 0\n",
        &[],
    ),
    (
        "a_numeric_domain_does_not_erase_a_dependent_bound",
        "run:\n    region: Region(4, 2)\n    value: region.at(3)\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "a_known_index_can_satisfy_a_dependent_bound",
        "run:\n    region: Region(4, 2)\n    value: region.at(1)\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "clearing_a_receiver_refutes_its_previous_guard",
        "clear(region mut Region):\n    region.initialized :: replace(region.initialized, 0)\n    return\nread(region mut Region) Count:\n    when region.initialized_count > 0 then\n        clear(region)\n        return region.last\n    return 0\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "unknown_receiver_members_are_rejected",
        "read(region ref Region and region.missing > 0) Count: return 0\n",
        &["type-constraint-unknown-name"],
    ),
    (
        "constrained_queries_are_not_silently_assumed_callable",
        "read(region ref Region and region.last > 0) Count: return 0\n",
        &["type-constraint-unproven-term"],
    ),
    (
        "a_loop_guard_is_reestablished_for_each_iteration",
        "clear(region mut Region):\n    region.initialized :: replace(region.initialized, 0)\n    return\nread(region mut Region):\n    while region.initialized_count > 0 do\n        value: region.last\n        value.release\n        clear(region)\n    return\n",
        &[],
    ),
    (
        "clearing_the_receiver_inside_a_loop_refutes_the_guard",
        "clear(region mut Region):\n    region.initialized :: replace(region.initialized, 0)\n    return\nread(region mut Region):\n    while region.initialized_count > 0 do\n        clear(region)\n        value: region.last\n        value.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "receiver_preconditions_support_a_checked_decrement",
        "Region has\n    pop(self mut and self.initialized_count > 0) Count:\n        count: self.initialized.as_num\n        index: count - 1\n        self.initialized :: replace(self.initialized, index as Count)\n        return 0\nread(region mut Region):\n    while region.initialized_count > 0 do\n        value: region.pop\n        value.release\n    return\n",
        &[],
    ),
    (
        "a_successful_result_match_preserves_owned_record_field_facts",
        "Failure has release(self): return\nproduce(marker Count) region Region thus region.initialized_count = 1 or Failure: return Region(4, 1)\nrun:\n    region: when produce(0) is\n        Region(value) then value\n        Failure(error) then\n            error.release\n            return\n    value: region.last\n    value.release\n    region.release\n    return\n",
        &[],
    ),
];

#[test]
fn receiver_precondition_cases() {
    for &(name, body, expected) in CASES {
        expect_flaws_with_named(TYPES, body, expected, name);
    }
}

#[test]
fn arithmetic_in_a_dependent_bound_substitutes_its_receiver() {
    expect_flaws_with(
        TYPES,
        "read(region ref Region, index Count and index < self.initialized_count + 1) Count: return index\n",
        &["type-constraint-unknown-name"],
    );
    expect_flaws_with(
        TYPES,
        "read(region ref Region, index Count and index < region.initialized_count + 1) Count: return index\nrun:\n    region: Region(4, 2)\n    value: read(region, 2)\n    value.release\n    region.release\n    return\n",
        &[],
    );
}

#[test]
fn generic_receiver_queries_preserve_count_units() {
    let source = "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\nA is in 0 to 255\nA has release(self): return\nB is in 0 to 255\nB has release(self): return\n#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has Numeric\n    release(self):\n        return\nRegion(unit) has\n    capacity Count(unit)\n    initialized Count(unit) and <= capacity\n    initialized_count(self ref) Count(unit): return self.initialized.as_num as Count(unit)\n    last(self ref and self.initialized_count > 0) Count(unit): return 0\n    release(self):\n        return\nread(region ref Region(A), index Count(B) and < region.initialized_count) Count(B): return index\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = check(source);
    assert_eq!(
        flaw_codes(&typed),
        ["type-constraint-type-mismatch"],
        "{:?}",
        typed.all_flaws()
    );
    let typed = check(&source.replace("Count(B)", "Count(A)"));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn receiver_constraints_work_across_modules_and_respect_member_privacy() {
    for (member, expected) in [
        ("initialized_count", None),
        ("initialized", Some("type-private-member")),
    ] {
        for reversed in [false, true] {
            let model = TYPES.replace(
                "    initialized Count",
                "    #private\n    initialized Count",
            );
            let client = format!(
                "use sample.model Count, Region\nread(region ref Region and region.{member} > 0) Count: return 0\n"
            );
            let mut files = [("model", model.as_str()), ("client", client.as_str())];
            if reversed {
                files.reverse();
            }
            let output = package("sample", &files);
            let flaws: Vec<_> = output.iter().flat_map(|file| file.all_flaws()).collect();
            let codes: Vec<_> = flaws.iter().map(|(_, flaw)| flaw.code.slug()).collect();
            assert_eq!(codes, expected.into_iter().collect::<Vec<_>>(), "{flaws:?}");
        }
    }
}

#[test]
fn implicit_array_lengths_are_typed_and_instantiated_in_preconditions() {
    expect_flaws_with(
        TYPES,
        "bounded(values ref [Count; count] and count <= 8): return\nrun:\n    values: [1 as Count; 8]\n    bounded(values)\n    return\n",
        &[],
    );
    expect_flaws_with(
        TYPES,
        "bounded(values ref [Count; count] and count <= 8): return\nrun:\n    values: [1 as Count; 9]\n    bounded(values)\n    return\n",
        &["type-parameter-refinement-failed"],
    );
}

#[test]
fn portable_constraints_use_the_smallest_supported_index_width() {
    let signature = "pointer_bits := #index_bits(@())\nuse_size(value Count and value <= pointer_bits):\n    value.release\n    return\n";
    for (value, expected) in [
        (32, vec![]),
        (64, vec!["type-parameter-refinement-unproven"]),
    ] {
        let source = format!("{TYPES}{signature}run: return use_size({value})\n");
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let context = typecheck::transform::TransformCtx::with_package_compile_time_arc(
            std::sync::Arc::new(parsed.ast.clone()),
        );
        let typed = typecheck::transform::transform(&parsed.ast, typecheck::FileId(0), &context);
        assert_eq!(flaw_codes(&typed), expected, "{:?}", typed.all_flaws());
    }
}
