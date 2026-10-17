mod support;

use parser::query::SourceParseExt;
use support::{check, flaw_codes};

const MODEL: &str = "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\nCount is in 0 to 255\nCount has Numeric\n    release(self):\n        return\nreplace(previous Count, next Count) Count: return next\nRegion has\n    capacity Count\n    initialized Count and <= capacity\n    initialized_count(self ref) is in Count: return self.initialized.as_num\n    pop(self mut and self.initialized_count > 0) Count:\n        count: self.initialized.as_num\n        self.initialized :: replace(self.initialized, (count - 1) as Count)\n        return 0\n    release(self):\n        return\nWrapper has\n    region Region and region.initialized_count = 1\n    take(self) Count:\n        Wrapper(region) := self\n        owned: region\n        value: owned.pop\n        owned.release\n        return value\n    release(self):\n        Wrapper(region) := self\n        region.release\n        return\n";

fn expect(body: &str, expected: &[&str]) {
    let source = format!("{MODEL}{body}");
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = check(&source);
    assert_eq!(flaw_codes(&typed), expected, "{:?}", typed.all_flaws());
}

#[test]
fn nested_invariants_survive_destructuring_and_owned_moves() {
    expect(
        "run:\n    wrapper: Wrapper(Region(4, 1))\n    value: wrapper.take\n    value.release\n    return\n",
        &[],
    );
}

#[test]
fn construction_requires_the_nested_invariant() {
    expect(
        "run:\n    wrapper: Wrapper(Region(4, 0))\n    wrapper.release\n    return\n",
        &["dep-refinement-failed"],
    );
}

#[test]
fn an_unknown_nested_state_cannot_construct_the_wrapper() {
    expect(
        "wrap(region Region) Wrapper: return Wrapper(region)\n",
        &["dep-refinement-unproven"],
    );
}

#[test]
fn a_mutable_reference_cannot_escape_a_nested_invariant() {
    expect(
        "expose(region mut Region) mut Region: return mut region\nborrow(wrapper mut Wrapper) mut Region: return expose(wrapper.region)\n",
        &["dep-refinement-unproven"],
    );
}

#[test]
fn replacing_a_nested_field_requires_preserving_its_invariant() {
    expect(
        "replace_region(previous Region, next Region) Region:\n    previous.release\n    return next\nreset(wrapper mut Wrapper):\n    wrapper.region :: replace_region(wrapper.region, Region(4, 0))\n    return\n",
        &["dep-refinement-failed"],
    );
}

#[test]
fn a_returned_records_verified_state_is_available_to_the_caller() {
    expect(
        "make_region(capacity Count and capacity > 0) Region:\n    region: Region(capacity.as_num as Count, 0)\n    return region thus region.initialized_count = 0\nrun:\n    region: make_region(4)\n    wrapper: Wrapper(region)\n    wrapper.release\n    return\n",
        &["dep-refinement-failed"],
    );
}

#[test]
fn returned_record_guarantees_survive_different_exit_bindings() {
    expect(
        "make_region(capacity Count and capacity > 0) Region:\n    when capacity.as_num > 4 then\n        first: Region(capacity.as_num as Count, 1)\n        return first thus first.initialized_count = 1\n    second: Region(capacity.as_num as Count, 1)\n    return second thus second.initialized_count = 1\nrun:\n    region: make_region(4)\n    wrapper: Wrapper(region)\n    wrapper.release\n    return\n",
        &[],
    );
}

#[test]
fn returned_record_guarantees_cannot_describe_a_different_record() {
    expect(
        "make_region(capacity Count and capacity > 0) Region:\n    checked: Region(capacity.as_num as Count, 1)\n    unchecked: Region(capacity.as_num as Count, 0)\n    checked.release\n    return unchecked thus checked.initialized_count = 1\nrun:\n    region: make_region(4)\n    wrapper: Wrapper(region)\n    wrapper.release\n    return\n",
        &["dep-refinement-unproven"],
    );
}

#[test]
fn forwarding_an_owned_record_preserves_its_state_across_other_cleanup() {
    expect(
        "forward(previous Region, next Region) Region:\n    previous.release\n    return next\nrun:\n    region: forward(Region(4, 0), Region(4, 1))\n    wrapper: Wrapper(region)\n    wrapper.release\n    return\n",
        &[],
    );
}

#[test]
fn a_conditional_return_does_not_publish_forwarding_from_the_final_exit() {
    expect(
        "forward(previous Region, next Region) Region:\n    when previous.initialized_count > 0 then\n        next.release\n        return previous\n    previous.release\n    return next\nwrap(previous Region, next Region) Wrapper: return Wrapper(forward(previous, next))\n",
        &["dep-refinement-unproven"],
    );
}

#[test]
fn getter_bounds_authorize_integer_conversions_without_accepting_domain_holes() {
    for (target, accepted) in [("Count", true), ("Nonzero", false)] {
        let source = format!(
            "{MODEL}Wide is in 0 to 65535\nWide has release(self): return\nNonzero is in 0 to 255 and /= 0\nNonzero has release(self): return\nconvert(region ref Region, index ref Wide and index < region.initialized_count) {target}: return (position is in Wide: index) as {target}\n"
        );
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let typed = check(&source);
        if accepted {
            assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        } else {
            assert!(
                flaw_codes(&typed)
                    .iter()
                    .any(|code| code.starts_with("type-integer-narrowing-")),
                "{:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn moving_a_verified_returned_record_into_a_field_preserves_its_facts() {
    expect(
        "make_region(capacity Count and capacity > 0) Region:\n    region: Region(capacity.as_num as Count, 1)\n    return region thus region.initialized_count = 1\nreplace_region(previous Region, next Region) Region:\n    previous.release\n    return next\nreset(wrapper mut Wrapper):\n    wrapper.region :: replace_region(wrapper.region, make_region(4))\n    return thus wrapper.region.initialized_count = 1\n",
        &[],
    );
}
