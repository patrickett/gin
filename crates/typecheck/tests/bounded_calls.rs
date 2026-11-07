//! A type parameter declared `element has Trait` can have the trait's methods called on it:
//! `element.release(value)` ends an owned `value`, and nothing else can.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const RELEASE: &str = "Release has\n    release(self)\n\n";

#[test]
fn a_bounded_parameter_can_be_released_through_its_trait() {
    check(&format!(
        "{RELEASE}dispose(element has Release, value element):\n    element.release(value)\n    return\n"
    ))
    .unwrap();
}

#[test]
fn without_the_call_an_owned_generic_value_is_unconsumed() {
    assert_eq!(
        check(&format!(
            "{RELEASE}dispose(element has Release, value element):\n    return\n"
        )),
        Err(SourceCheckError::Unconsumed)
    );
}

#[test]
fn a_released_value_cannot_be_used_again() {
    assert_eq!(
        check(&format!(
            "{RELEASE}twice(element has Release, value element):\n    element.release(value)\n    element.release(value)\n    return\n"
        )),
        Err(SourceCheckError::UseAfterMove)
    );
}

#[test]
fn an_unbounded_parameter_has_no_methods() {
    assert!(check(&format!(
        "{RELEASE}dispose(element, value element):\n    element.release(value)\n    return\n"
    ))
    .is_err());
}

#[test]
fn a_type_declaration_bounds_its_parameter_for_its_methods() {
    check(&format!(
        "{RELEASE}Wrapped(element has Release) has\n    value element\n\n    release(self):\n        Wrapped(value) := self\n        element.release(value)\n        return\n"
    ))
    .unwrap();
}
