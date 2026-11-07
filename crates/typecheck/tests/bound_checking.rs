//! A type argument given for a bounded parameter must provide the trait: the check is made at
//! every type application and every call, where the argument is written.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const PRELUDE: &str = "Release has
    release(self)

#unrestricted
Good is in 0 to 9
Good has Release
    release(self): return

#unrestricted
Bad is in 0 to 9

Wrapped(element has Release) has
    value element

    release(self):
        Wrapped(value) := self
        element.release(value)
        return

";

#[test]
fn a_type_that_provides_the_trait_is_accepted() {
    check(&format!("{PRELUDE}hold(value Good) Wrapped(Good): Wrapped(value: value)\n")).unwrap();
}

#[test]
fn a_type_that_does_not_is_rejected_where_it_is_applied() {
    assert_eq!(
        check(&format!("{PRELUDE}hold(value Bad) Wrapped(Bad): Wrapped(value: value)\n")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_call_that_supplies_a_type_argument_is_checked() {
    let dispose = "dispose(element has Release, value element):\n    element.release(value)\n    return\n\n";
    check(&format!("{PRELUDE}{dispose}use_it(value Good): dispose(Good, value)\n")).unwrap();
    assert_eq!(
        check(&format!("{PRELUDE}{dispose}use_it(value Bad): dispose(Bad, value)\n")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_type_parameter_with_the_same_bound_provides_it() {
    check(&format!(
        "{PRELUDE}pass_on(element has Release, value element):\n    dispose_it(element, value)\n\ndispose_it(element has Release, value element):\n    element.release(value)\n    return\n"
    ))
    .unwrap();
}

#[test]
fn an_unbounded_parameter_does_not_provide_a_bound() {
    assert_eq!(
        check(&format!(
            "{PRELUDE}pass_on(element, value element):\n    dispose_it(element, value)\n\ndispose_it(element has Release, value element):\n    element.release(value)\n    return\n"
        )),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_bare_integer_range_cannot_stand_for_a_bounded_parameter() {
    assert!(check(&format!(
        "{PRELUDE}dispose_it(element has Release, value element):\n    element.release(value)\n    return\n\nuse_it(value in 0 to 9): dispose_it(0 to 9, value)\n"
    ))
    .is_err());
}
