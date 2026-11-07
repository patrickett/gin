//! `deref value` copies what a borrow refers to; `ref place` and `mut place` create borrows.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.deref", &source, &[], Limits::default())
}

const STATS: &str = "\
#unrestricted
Stats has
    count in 0 to 9
";

#[test]
fn deref_copies_a_copyable_record_out_of_a_ref() {
    let source = format!("{STATS}\ncopy(value ref Stats) Stats: return deref value\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source).err());
}

/// A copy of a linear value would duplicate its obligation, so it cannot be moved out.
#[test]
fn deref_of_a_linear_referent_is_refused() {
    let source = "\
Token has
    number in 0 to 9

take(value ref Token) Token: return deref value
";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::MoveOutOfBorrow);
}

#[test]
fn deref_of_a_number_is_the_number() {
    let source = "read(value ref in 0 to 9) in 0 to 9: return deref value\n";
    assert!(check(source).is_ok(), "{:?}", check(source).err());
}

/// The copy is its own value: the original is untouched and both can be used.
#[test]
fn the_copy_and_the_original_are_both_usable() {
    let source = format!(
        "{STATS}\ntwice(value ref Stats) Stats:\n    first: deref value\n    second: deref value\n    return first\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source).err());
}

/// A borrowed number has no owner to tell it from a plain one, so `deref` of a number is the
/// number whether or not it came through a borrow. Only an owned record is held to the rule.
#[test]
fn deref_of_a_plain_number_is_that_number() {
    let source = "read() in 0 to 9: return deref 5\n";
    assert!(check(source).is_ok(), "{:?}", check(source).err());
}

/// Borrowing a field of a record and copying it out.
#[test]
fn a_borrow_expression_creates_a_borrow_of_a_place() {
    let source = format!(
        "{STATS}\ncount_of(value ref Stats) in 0 to 9:\n    seen: ref value.count\n    return deref seen\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source).err());
}
