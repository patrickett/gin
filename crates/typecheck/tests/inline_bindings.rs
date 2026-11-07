//! `(name in Domain: value)` is the value, checked in the domain; the name is not kept.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

#[test]
fn a_value_that_fits_the_domain_passes() {
    check("widen(n in 0 to 50) in 0 to 100:\n    (value in 0 to 100: n + 3)\n").unwrap();
}

#[test]
fn a_value_that_may_not_fit_is_rejected() {
    assert!(check("narrow(n in 0 to 50) in 0 to 20:\n    (value in 0 to 20: n)\n").is_err());
}

#[test]
fn the_bound_name_does_not_leak() {
    assert!(
        check("leak(n in 0 to 50) in 0 to 100:\n    (value in 0 to 100: n)\n    value\n").is_err()
    );
}

#[test]
fn it_works_as_a_local_initializer_and_inside_arithmetic() {
    check("bump(n in 0 to 50) in 0 to 255:\n    next: ((m in 0 to 100: n) + 1)\n    next\n").unwrap();
}
