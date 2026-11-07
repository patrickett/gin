//! A method of a generic type that takes `self` is generic over the type's parameters, which a
//! call reads from the receiver's type.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const PAIR: &str = "Pair(element) has
    left element
    right element

    swap(self) Pair(element):
        Pair(left, right) := self
        Pair(left: right, right: left)

";

#[test]
fn a_method_of_a_generic_type_checks_and_can_be_called() {
    check(&format!(
        "{PAIR}flip(element, p Pair(element)) Pair(element): p.swap()\n"
    ))
    .unwrap();
}

#[test]
fn the_receiver_is_moved_by_a_method_that_takes_self() {
    assert_eq!(
        check(&format!(
            "{PAIR}twice(element, p Pair(element)) Pair(element):\n    first: p.swap()\n    p.swap()\n"
        )),
        Err(SourceCheckError::UseAfterMove)
    );
}

#[test]
fn a_method_result_keeps_the_receivers_type_arguments() {
    // The swapped pair is a Pair of the same element, not of something else.
    assert!(check(&format!(
        "{PAIR}Other has\nwrong(element, p Pair(element)) Other: p.swap()\n"
    ))
    .is_err());
}

#[test]
fn a_body_error_inside_a_generic_method_is_found_where_it_is_written() {
    // Both fields are used, so nothing may be dropped: leaving one is an error.
    let source = "Pair(element) has
    left element
    right element

    first(self) element:
        Pair(left, right) := self
        left
";
    assert_eq!(check(source), Err(SourceCheckError::Unconsumed));
}
