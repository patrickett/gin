//! Refinement clauses beyond implicit comparisons: `and in L to U`.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<typecheck::CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict refinement fixture");
    check_file(&file, Limits::default())
}

#[test]
fn implicit_domain_refinement_accepts_an_interior_value() {
    check("f() in 0 to 255:\n    y in 0 to 255 and in 100 to 200: 155\n    return y\n")
        .expect("155 lies inside 100 to 200");
}

#[test]
fn implicit_domain_refinement_rejects_an_exterior_value() {
    assert_eq!(
        check("f() in 0 to 255:\n    y in 0 to 255 and in 100 to 200: 55\n    return y\n"),
        Err(SourceCheckError::Unproved),
    );
}

#[test]
fn implicit_domain_refinement_is_conjunctive_with_earlier_clauses() {
    // The base domain admits 5, but the `and in` clause excludes it.
    assert_eq!(
        check("f() in 0 to 255:\n    y in 0 to 255 and > 10 and in 100 to 200: 5\n    return y\n"),
        Err(SourceCheckError::Unproved),
    );
    check("f() in 0 to 255:\n    y in 0 to 255 and > 10 and in 100 to 200: 155\n    return y\n")
        .expect("155 satisfies every clause");
}
