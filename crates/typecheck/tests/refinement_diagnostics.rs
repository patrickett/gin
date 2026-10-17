mod support;

use diagnostic::{Category, Diagnostic};

const LOOKUP: &str = include_str!("../../test-fixtures/fixtures/runtime_bounded_lookup.gin");
const REGION: &str = r#"
Numeric has
    as_num(self ref) is in Self: return (number is in Self: self)
Count is in 0 to 255
Count has Numeric
    release(self):
        return
replace(previous Count, next Count) Count: return next
Region has
    capacity Count
    initialized Count and <= capacity
    release(self):
        return
"#;

fn single_flaw(source: &str, code: &str) -> Diagnostic {
    let typed = support::check_parsed(source);
    let flaws = typed.all_flaws();
    assert_eq!(flaws.len(), 1, "{flaws:?}\n{source}");
    let flaw = flaws[0].1;
    assert_eq!(flaw.code.slug(), code, "{flaw:?}");
    assert_eq!(flaw.category, Category::Flaw);
    flaw.clone()
}

fn accepted(source: &str) {
    let typed = support::check_parsed(source);
    assert!(
        typed.all_flaws().is_empty(),
        "{:?}\n{source}",
        typed.all_flaws()
    );
}

#[test]
fn an_unproven_argument_explains_the_obligation_and_both_recovery_paths() {
    let flaw = single_flaw(
        &format!("{LOOKUP}\nread(view ref View(Byte), index Count) Byte: return at(view, index)\n"),
        "type-parameter-refinement-unproven",
    );
    assert_eq!(
        flaw.message,
        "cannot prove refinement `index < view.length` for argument `index`"
    );
    assert_eq!(flaw.arg("name"), Some("index"));
    let help = flaw.help.unwrap();
    assert!(
        help.contains("check this condition before the call"),
        "{help}"
    );
    assert!(help.contains("caller's parameter refinement"), "{help}");

    accepted(&format!(
        "{LOOKUP}\nread(view ref View(Byte), index Count) Byte:\n    when index.as_num < view.length.as_num then\n        return at(view, index)\n    return 0\n"
    ));
    accepted(&format!(
        "{LOOKUP}\nread(view ref View(Byte), index Count and < view.length) Byte: return at(view, index)\n"
    ));
}

#[test]
fn changing_a_checked_argument_requires_checking_its_current_value() {
    let before = r#"
read(view ref View(Byte), index Count, next Count) Byte:
    position: index
    when position.as_num >= view.length.as_num then
        return 0
    position :: Count.replace(position, next)
"#;
    let flaw = single_flaw(
        &format!("{LOOKUP}\n{before}    return at(view, position)\n"),
        "type-parameter-refinement-unproven",
    );
    assert!(
        flaw.help
            .as_deref()
            .unwrap()
            .contains("check again after changing any value")
    );
    accepted(&format!(
        "{LOOKUP}\n{before}    when position.as_num >= view.length.as_num then
        return 0
    return at(view, position)
"
    ));
}

#[test]
fn an_unproven_constructor_explains_how_to_check_field_values() {
    let flaw = single_flaw(
        &format!(
            "{REGION}\nmake(capacity Count, initialized Count) Region:              return Region(capacity, initialized)\n"
        ),
        "dep-refinement-unproven",
    );
    assert_eq!(
        flaw.message,
        "cannot prove refinement `initialized <= capacity` for field `initialized`"
    );
    assert!(
        flaw.help
            .as_deref()
            .unwrap()
            .contains("before constructing the record")
    );
    accepted(&format!(
        "{REGION}\nmake(capacity Count, initialized Count):\n    when initialized.as_num <= capacity.as_num then\n        region: Region(capacity, initialized)\n        region.release\n        return\n    return\n"
    ));
    accepted(&format!(
        "{REGION}\nmake(capacity Count, initialized Count and <= capacity) Region:          return Region(capacity, initialized)\n"
    ));
}

#[test]
fn an_unproven_replacement_explains_which_constraint_must_survive() {
    let flaw = single_flaw(
        &format!(
            "{REGION}\nset(region mut Region, value Count):
    region.initialized :: replace(region.initialized, value)
    return
"
        ),
        "dep-refinement-unproven",
    );
    assert_eq!(
        flaw.message,
        "cannot prove refinement `initialized <= capacity` after replacement of `initialized`"
    );
    assert_eq!(flaw.arg("name"), Some("initialized"));
    assert!(
        flaw.help
            .as_deref()
            .unwrap()
            .contains("using the replacement value and the current record fields")
    );
    accepted(&format!(
        "{REGION}\nset(region mut Region, value Count):\n    when value.as_num > region.capacity.as_num then\n        return\n    region.initialized :: replace(region.initialized, value)\n    return\n"
    ));
}

#[test]
fn replacing_a_dependency_names_the_affected_constraint() {
    let flaw = single_flaw(
        &format!(
            "{REGION}\nshrink(region mut Region, value Count):
    region.capacity :: replace(region.capacity, value)
    return
"
        ),
        "dep-refinement-unproven",
    );
    assert_eq!(
        flaw.message,
        "cannot prove refinement `initialized <= capacity` after replacement of `capacity`"
    );
    accepted(&format!(
        "{REGION}\nshrink(region mut Region, value Count):\n    when value.as_num < region.initialized.as_num then\n        return\n    region.capacity :: replace(region.capacity, value)\n    return\n"
    ));
}

#[test]
fn a_record_mutation_requires_rechecking_dependencies() {
    let before = r#"
set(region mut Region, value Count):
    when value.as_num > region.capacity.as_num then
        return
    region.initialized :: replace(region.initialized, 0)
    region.capacity :: replace(region.capacity, 0)
"#;
    let replacement = "    region.initialized :: replace(region.initialized, value)\n    return\n";
    let flaw = single_flaw(
        &format!("{REGION}\n{before}{replacement}"),
        "dep-refinement-unproven",
    );
    assert!(
        flaw.help
            .as_deref()
            .unwrap()
            .contains("check again after changing any dependency")
    );
    accepted(&format!(
        "{REGION}\n{before}    when value.as_num > region.capacity.as_num then
        return
{replacement}"
    ));
}

#[test]
fn mutable_field_access_suggests_checked_replacement_through_the_parent() {
    let flaw = single_flaw(
        &format!("{REGION}\nborrow(region mut Region) mut Count: return mut region.initialized\n"),
        "dep-refinement-unproven",
    );
    assert!(flaw.message.contains("parent record's constraints"));
    assert!(
        flaw.help
            .as_deref()
            .unwrap()
            .contains("replace the field through its parent record")
    );
    accepted(&format!(
        "{REGION}\nset(region mut Region, value Count):\n    when value.as_num > region.capacity.as_num then\n        return\n    region.initialized :: replace(region.initialized, value)\n    return\n"
    ));
}

#[test]
fn an_unproven_postcondition_suggests_a_guard_or_input_refinement() {
    let flaw = single_flaw(
        "verify(value is in 0 to 255):\n    return thus value = 0\n",
        "type-postcondition-unproven",
    );
    assert_eq!(
        flaw.message,
        "cannot prove return postcondition `value = 0`"
    );
    let help = flaw.help.unwrap();
    assert!(
        help.contains("establish this condition before returning"),
        "{help}"
    );
    assert!(help.contains("function's parameter refinements"), "{help}");
    accepted(
        "verify(value is in 0 to 255):\n    when value = 0 then\n        return thus value = 0\n    return\n",
    );
    accepted("verify(value is in 0 to 255 and value = 0):\n    return thus value = 0\n");
}

#[test]
fn a_postcondition_guard_must_use_the_current_value_after_mutation() {
    let before = r#"
verify(value mut is in 0 to 255, next is in 0 to 255):
    decision: value = 0
    value :: next
    when decision then
"#;
    let flaw = single_flaw(
        &format!("{before}        return thus value = 0\n    return\n"),
        "type-postcondition-unproven",
    );
    assert!(
        flaw.help
            .as_deref()
            .unwrap()
            .contains("check the current values after any mutation")
    );
    accepted(&format!(
        "{before}        when value = 0 then
            return thus value = 0
        return
    return
"
    ));
}

#[test]
fn known_false_refinements_keep_their_failure_diagnostics() {
    for (source, code) in [
        (
            format!("{LOOKUP}\nread(view ref View(Byte)) Byte: return at(view, 255)\n"),
            "type-parameter-refinement-failed",
        ),
        (
            format!("{REGION}\nrun:\n    region: Region(4, 5)\n    region.release\n    return\n"),
            "dep-refinement-failed",
        ),
        (
            format!(
                "{REGION}\nrun:
    region: Region(4, 0)
    region.initialized :: replace(region.initialized, 5)
    region.release
    return
"
            ),
            "dep-refinement-failed",
        ),
        (
            "verify(value mut is in 0 to 255):\n    value :: 1\n    return thus value = 0\n".into(),
            "type-postcondition-failed",
        ),
    ] {
        let flaw = single_flaw(&source, code);
        assert_eq!(flaw.help, None, "{flaw:?}");
    }
}

#[test]
fn compound_refinements_use_readable_conditions() {
    let flaw = single_flaw(
        &format!(
            "{REGION}\nrequire(value Count and > 0 and < 10) Count: return value\ncall(value Count) Count: return require(value)\n"
        ),
        "type-parameter-refinement-unproven",
    );
    assert_eq!(
        flaw.message,
        "cannot prove refinement `value > 0 and value < 10` for argument `value`"
    );
    let flaw = single_flaw(
        &format!(
            "{REGION}\nrequire(value Count and value = 1 or value = 3) Count: return value\ncall(value Count) Count: return require(value)\n"
        ),
        "type-parameter-refinement-unproven",
    );
    assert!(flaw.message.contains("(value = 1) or (value = 3)"));
    assert!(flaw.help.is_some());
}
