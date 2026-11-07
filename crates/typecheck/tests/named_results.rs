//! `f(..) first T, second U` returns a record with those fields: `return a, b` builds it and
//! the caller reads `r.first`.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const PAIR: &str = "Pair has
    left in 0 to 9
    right in 0 to 9

    split(self) first in 0 to 9, second in 0 to 9:
        Pair(left, right) := self
        return left, right

";

#[test]
fn several_named_results_are_returned_and_read_by_name() {
    check(&format!(
        "{PAIR}take(pair Pair) in 0 to 9:\n    parts: pair.split()\n    return parts.second\n"
    ))
    .unwrap();
}

#[test]
fn returning_the_wrong_number_of_values_is_a_type_error() {
    assert_eq!(
        check(&PAIR.replace("return left, right", "return left")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_value_outside_the_declared_range_is_rejected() {
    assert!(check(
        &PAIR.replace("second in 0 to 9:", "second in 0 to 3:")
    )
    .is_err());
}

#[test]
fn an_unknown_result_name_is_rejected() {
    assert!(check(&format!(
        "{PAIR}take(pair Pair) in 0 to 9:\n    parts: pair.split()\n    return parts.third\n"
    ))
    .is_err());
}

#[test]
fn two_results_may_not_share_a_name() {
    assert_eq!(
        check(&PAIR.replace("second in 0 to 9:", "first in 0 to 9:")),
        Err(SourceCheckError::Duplicate)
    );
}
