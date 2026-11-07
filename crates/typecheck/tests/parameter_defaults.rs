//! A type parameter may have a default: `Holder(element, alloc: Global)` applied as
//! `Holder(Byte)` means `Holder(Byte, Global)`, and an explicit argument overrides it.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const TYPES: &str = "#unrestricted
Global is in 0 to 0
#unrestricted
Other is in 0 to 0

Holder(element, alloc: Global) has
    value element

";

#[test]
fn an_omitted_argument_takes_the_default() {
    check(&format!(
        "{TYPES}make(element, value element) Holder(element): Holder(value: value)\n"
    ))
    .unwrap();
}

#[test]
fn an_explicit_argument_overrides_the_default() {
    check(&format!(
        "{TYPES}make(element, value element) Holder(element, Other): Holder(value: value)\n"
    ))
    .unwrap();
}

#[test]
fn the_default_is_the_type_the_omitted_form_means() {
    // `Holder(element)` is `Holder(element, Global)`, which is not `Holder(element, Other)`.
    assert!(check(&format!(
        "{TYPES}wrong(element, h Holder(element)) Holder(element, Other): h\n"
    ))
    .is_err());
}

#[test]
fn a_parameter_without_a_default_must_be_given() {
    assert!(check(&format!("{TYPES}bad() Holder: Holder(value: 1)\n")).is_err());
}

#[test]
fn a_default_must_name_a_type() {
    assert!(check("Bad(element, extra: Missing) has\n    value element\n").is_err());
}
