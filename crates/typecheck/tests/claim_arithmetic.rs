//! Claims may add, subtract and multiply parameters and ask the size of a type:
//! `count * #size(element) <= limit`. At a concrete type the size is a known number and the claim
//! is proved or refused; for a type parameter it is only some number from 1 up, so a claim that
//! depends on it must be restated by the caller.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const CELL: &str = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Wide is in 0 to 4294967295
limit := 1000

Cell(element) has
    count in 0 to 1000 and count * #size(element) <= limit

    release(self):
        Cell(count) := self
        return

";

fn with(tail: &str) -> String {
    format!("{CELL}{tail}\n")
}

#[test]
fn a_size_claim_is_proved_at_a_concrete_type() {
    check(&with("ok() Cell(Byte): return Cell(count: 900)")).unwrap();
    check(&with("fine() Cell(Wide): return Cell(count: 250)")).unwrap();
}

#[test]
fn a_size_claim_that_fails_at_the_type_is_refused() {
    // Four bytes each: 900 of them are 3600.
    assert_eq!(
        check(&with("bad() Cell(Wide): return Cell(count: 900)")),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn for_a_type_parameter_the_claim_must_be_restated() {
    // The size of `element` is some number from 1 up: 10 of them may exceed the limit.
    assert_eq!(
        check(&with("make(element) Cell(element): return Cell(count: 10)")),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn arithmetic_over_parameters_is_checked_at_a_call() {
    let text = "limit := 100\nspace(a in 0 to 100, b in 0 to 100 and a + b <= limit) in 0 to 100: return a\n\nuse_it() in 0 to 100: return space(30, 70)\n";
    check(text).unwrap();
    let wrong = "limit := 100\nspace(a in 0 to 100, b in 0 to 100 and a + b <= limit) in 0 to 100: return a\n\nuse_it() in 0 to 100: return space(30, 71)\n";
    assert_eq!(check(wrong), Err(SourceCheckError::Unproved));
}

#[test]
fn a_difference_and_a_product_of_constants_are_numbers() {
    check("cap := 1024\nf(n in 0 to 2000 and n <= cap - 4) in 0 to 1020: return n\n").unwrap();
    check("f(n in 0 to 2000 and n <= 8 * 100) in 0 to 800: return n\n").unwrap();
}
