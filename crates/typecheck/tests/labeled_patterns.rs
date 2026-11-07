//! `binder: field` in a pattern binds the local `binder` to the field named `field`; a bare
//! name beside labels means the field of that name. Every field is named exactly once.
use typecheck::{Limits, SourceCheckError, check_file};

const PAIR: &str = "Pair has left in 0 to 9, right in 0 to 9\n\n";

fn check(function: &str) -> Result<(), SourceCheckError> {
    let text = format!("{PAIR}{function}");
    let source = parser::parse_strict(&text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

#[test]
fn labels_pick_fields_in_any_order() {
    check("flip(pair Pair) in 0 to 99:\n    Pair(second: right, first: left) := pair\n    first * 10 + second\n").unwrap();
}

#[test]
fn a_bare_name_beside_a_label_is_the_field_of_that_name() {
    check("mixed(pair Pair) in 0 to 99:\n    Pair(left, other: right) := pair\n    left * 10 + other\n").unwrap();
}

#[test]
fn an_unknown_field_is_rejected() {
    assert!(check("bad(pair Pair) in 0 to 9:\n    Pair(a: left, b: middle) := pair\n    a\n").is_err());
}

#[test]
fn a_field_named_twice_is_rejected() {
    assert_eq!(
        check("twice(pair Pair) in 0 to 9:\n    Pair(a: left, b: left) := pair\n    a\n"),
        Err(SourceCheckError::Duplicate)
    );
}

#[test]
fn every_field_must_be_named() {
    assert!(check("short(pair Pair) in 0 to 9:\n    Pair(a: left) := pair\n    a\n").is_err());
}
