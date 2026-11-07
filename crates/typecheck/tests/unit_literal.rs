//! `()` is the unit value: an arm value or a tail expression of a function that returns nothing.
use typecheck::{Limits, SourceCheckError, check_file_diagnostic};

fn run(text: &str) -> Result<(), SourceCheckError> {
    let file = parser::parse_strict(text).unwrap_or_else(|d| panic!("{text}\n{d:?}"));
    check_file_diagnostic(&file, Limits::default()).map(|_| ()).map_err(|failure| failure.error)
}

const T: &str = "#unrestricted\nT is A or B\n\n";

#[test]
fn unit_is_an_arm_value_and_a_tail() {
    assert_eq!(run(&format!("{T}f(t T): return when t is\n    A then ()\n    B then ()\n")), Ok(()));
    assert_eq!(run(&format!("{T}g(): return ()\n")), Ok(()));
    // Beside an empty call and the `()` result annotation, which keep their meaning.
    assert_eq!(run(&format!("{T}g() (): return ()\n\nh(): return g()\n")), Ok(()));
}

#[test]
fn unit_where_a_value_is_required_is_refused() {
    // A unit arm in a function that returns a number.
    assert_eq!(run(&format!("{T}f(t T) in 0 to 9: return when t is\n    A then ()\n    B then 1\n")), Err(SourceCheckError::Type));
    assert!(run(&format!("{T}f() in 0 to 9: return ()\n")).is_err());
    // A unit value bound and used as a number.
    assert!(run(&format!("{T}f() in 0 to 9:\n    u: ()\n    return u + 1\n")).is_err());
}
