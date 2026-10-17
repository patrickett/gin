mod support;

use support::{expect_flaws, package};

const COUNT: &str = "Count is in 0 to 255\nCount has release(self): return\n";

#[test]
fn unknown_constraint_dependencies_are_rejected_without_construction() {
    expect_flaws(
        &format!(
            "{COUNT}Region has\n    capacity Count\n    initialized Count and <= missing\n    release(self):\n        return\n"
        ),
        &["type-constraint-unknown-name"],
    );
}

#[test]
fn resource_fields_cannot_be_used_as_numeric_bounds() {
    expect_flaws(
        &format!(
            "{COUNT}Resource has\n    value Count\n    release(self):\n        return\nRegion has\n    capacity Resource\n    initialized Count and <= capacity\n    release(self):\n        self.capacity.release\n        return\n"
        ),
        &["type-constraint-noninteger"],
    );
}

#[test]
fn numeric_bounds_cannot_constrain_a_resource_value() {
    expect_flaws(
        &format!(
            "{COUNT}Resource has\n    value Count\n    release(self):\n        return\nRegion has\n    capacity Count\n    resource Resource and <= capacity\n    release(self):\n        self.resource.release\n        return\n"
        ),
        &["type-constraint-noninteger"],
    );
}

fn units(initialized: &str) -> String {
    format!(
        "A is in 0 to 255\nA has release(self): return\nB is in 0 to 255\nB has release(self): return\n#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has release(self): return\nRegion has\n    capacity Count(A)\n    initialized Count({initialized}) and <= capacity\n    release(self):\n        return\nrun:\n    region: Region(4, 0)\n    region.release\n    return\n"
    )
}

#[test]
fn count_units_must_match_even_when_the_values_satisfy_the_bound() {
    expect_flaws(&units("B"), &["type-constraint-type-mismatch"]);
}

#[test]
fn matching_count_units_are_accepted() {
    expect_flaws(&units("A"), &[]);
}

#[test]
fn propositional_constraints_check_each_operand() {
    expect_flaws(
        &format!(
            "{COUNT}Region has\n    capacity Count\n    initialized Count and initialized <= missing\n    release(self):\n        return\n"
        ),
        &["type-constraint-unknown-name"],
    );
}

#[test]
fn comparison_literals_need_not_fit_the_field_domain() {
    expect_flaws(
        &format!(
            "{COUNT}Region has\n    initialized Count and < 256\n    release(self):\n        return\n"
        ),
        &[],
    );
}

#[test]
fn imported_constants_have_types_in_constraint_scope() {
    let model = format!("{COUNT}limit := 4\n");
    let client = "use sample.model Count, limit\nRegion has\n    initialized Count and <= limit\n    release(self):\n        return\nread(index Count and < limit) Count: return index\nrun:\n    value: read(2)\n    value.release\n    return\n";
    for reversed in [false, true] {
        let mut files = [("model", model.as_str()), ("client", client)];
        if reversed {
            files.reverse();
        }
        let output = package("sample", &files);
        let flaws: Vec<_> = output.iter().flat_map(|file| file.all_flaws()).collect();
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}
