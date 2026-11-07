//! A provided trait gives its methods' results: `Add(IntegerResult)` makes `IntegerResult` the
//! result of `add`. A method that leaves its result off takes it; one that writes a result
//! writes exactly it.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const LIBRARY: &str = "\
Bool is True or False

Add(result) has
    #infix('+')
    add(self, rhs Self) result

Eq has
    equals(self, rhs Self) Bool

Failure is Overflow
Sum is Number(Number) or Flaw(Failure)

#unrestricted
Number is in 0 to 100

";

fn with(tail: &str) -> String {
    format!("{LIBRARY}{tail}\n")
}

#[test]
fn an_omitted_result_is_the_traits() {
    check(&with(
        "Number has Add(Sum)\n    add(self in Self, rhs in Self):\n        total: self + rhs\n        return when total is in Number then Number(total as Self) else Flaw(Failure.Overflow)\n",
    ))
    .unwrap();
}

#[test]
fn a_result_the_trait_writes_outright_is_supplied_too() {
    check(&with(
        "Number has Eq\n    equals(self in Self, rhs in Self): return when self = rhs then True else False\n",
    ))
    .unwrap();
}

#[test]
fn the_written_result_must_be_the_traits() {
    // Written exactly, it is accepted.
    check(&with(
        "Number has Add(Sum)\n    add(self in Self, rhs in Self) Sum:\n        total: self + rhs\n        return when total is in Number then Number(total as Self) else Flaw(Failure.Overflow)\n",
    ))
    .unwrap();
    // Written as something else, it is not what a caller of the trait sees.
    assert_eq!(
        check(&with(
            "Number has Add(Sum)\n    add(self in Self, rhs in Self) Number:\n        return (self + rhs) as Self\n"
        )),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn the_body_is_checked_against_the_supplied_result() {
    // `Bool` is the result, a number is not.
    assert_eq!(
        check(&with(
            "Number has Eq\n    equals(self in Self, rhs in Self): return self\n"
        )),
        Err(SourceCheckError::Type)
    );
}
