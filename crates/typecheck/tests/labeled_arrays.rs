//! `[a: x, b: y]` builds a record: the expected record when one is known, else the one record
//! with exactly those field names, else an anonymous record. Labels name fields in any order.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const TYPES: &str = "\
#unrestricted
Reg has
    arch Text
    index in 0 to 255

#unrestricted
Pair has
    left Reg
    right Reg

";

fn with(tail: &str) -> String {
    format!("{TYPES}{tail}\n")
}

#[test]
fn the_expected_record_names_the_type() {
    check(&with("a() Reg: [arch: 'x', index: 1]")).unwrap();
    // Labels are not positions.
    check(&with("b() Reg: [index: 2, arch: 'y']")).unwrap();
}

#[test]
fn nested_labeled_arrays_take_their_fields_types() {
    check(&with("c Pair: [left: [arch: 'x', index: 1], right: [arch: 'y', index: 2]]")).unwrap();
}

#[test]
fn without_an_expected_type_the_unique_record_is_found() {
    check(&with("d := [arch: 'x', index: 3]")).unwrap();
}

#[test]
fn labels_that_match_no_record_make_an_anonymous_record() {
    check(&with("e := [first: 1, second: 2]")).unwrap();
}

#[test]
fn a_wrong_label_or_value_is_refused() {
    assert!(check(&with("a() Reg: [arch: 'x', idx: 1]")).is_err());
    assert!(check(&with("a() Reg: [arch: 'x']")).is_err());
    assert!(check(&with("a() Reg: [arch: 'x', index: 1000]")).is_err());
    assert!(check(&with("a() Reg: [arch: 1, index: 1]")).is_err());
    // A label given twice is one field given twice.
    assert!(check(&with("a() Reg: [arch: 'x', arch: 'y']")).is_err());
}

#[test]
fn two_records_with_the_same_fields_are_ambiguous_without_a_type() {
    assert_eq!(
        check(&with("#unrestricted\nTwin has\n    arch Text\n    index in 0 to 255\n\nd := [arch: 'x', index: 3]")),
        Err(SourceCheckError::Duplicate)
    );
}
