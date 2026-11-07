//! `/ % & | ^ << >>` on integers: the checker knows the domain each result ranges over, and
//! refuses what would not be defined (a divisor that may be zero, a shift by a negative or by
//! too much). The guards that make them defined are the program's own.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

#[test]
fn division_truncates_toward_zero() {
    check("f(a in -7 to -7, b in 2 to 2) in -3 to -3: return a / b\n").unwrap();
    check("f(a in 7 to 7, b in -2 to -2) in -3 to -3: return a / b\n").unwrap();
    check("f(a in -100 to 100, b in 1 to 10) in -100 to 100: return a / b\n").unwrap();
    // The quotient of 100 by 1 is 100, so a tighter result is refused.
    assert_eq!(
        check("f(a in -100 to 100, b in 1 to 10) in -99 to 99: return a / b\n"),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_remainder_follows_its_dividend() {
    check("f(a in -7 to -7, b in 2 to 2) in -1 to -1: return a % b\n").unwrap();
    check("f(a in 7 to 7, b in -2 to -2) in 1 to 1: return a % b\n").unwrap();
    check("f(a in 0 to 100, b in 1 to 10) in 0 to 9: return a % b\n").unwrap();
    check("f(a in -100 to 100, b in 1 to 10) in -9 to 9: return a % b\n").unwrap();
}

#[test]
fn a_divisor_that_may_be_zero_is_not_proved() {
    assert_eq!(
        check("f(a in 0 to 100, b in 0 to 10) in 0 to 100: return a / b\n"),
        Err(SourceCheckError::Unproved)
    );
    assert_eq!(
        check("f(a in 0 to 100, b in -3 to 3) in -100 to 100: return a % b\n"),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_guard_that_cuts_zero_off_an_end_proves_the_divisor() {
    // `b > 0` returns, `b = 0` returns: what is left is below zero.
    check(
        "f(a in -100 to 100, b in -10 to 10) in -100 to 100:\n    when b > 0 then return a / b\n    when b = 0 then return 0\n    return a / b\n",
    )
    .unwrap();
    // Only the `= 0` guard: zero is inside the range, so the divisor is not proved.
    assert_eq!(
        check(
            "f(a in -100 to 100, b in -10 to 10) in -100 to 100:\n    when b = 0 then return 0\n    return a / b\n"
        ),
        Err(SourceCheckError::Unproved)
    );
    // ... but where zero is the bottom of the range it is cut off.
    check(
        "f(a in 0 to 100, b in 0 to 10) in 0 to 100:\n    when b = 0 then return 0\n    return a / b\n",
    )
    .unwrap();
}

#[test]
fn bit_operations_stay_inside_the_bits_they_are_given() {
    check("f(a in 0 to 255, b in 0 to 15) in 0 to 15: return a & b\n").unwrap();
    check("f(a in 0 to 255, b in 0 to 15) in 0 to 255: return a | b\n").unwrap();
    check("f(a in 0 to 255, b in 0 to 255) in 0 to 255: return a ^ b\n").unwrap();
    check("f(a in 12 to 12, b in 10 to 10) in 8 to 8: return a & b\n").unwrap();
    // A signed operand keeps its sign bit: the result fits the same signed width.
    check("f(a in -128 to 127, b in -128 to 127) in -128 to 127: return a ^ b\n").unwrap();
    // A mask cuts a negative number to a byte.
    check("f(a in -1000 to 1000) in 0 to 255: return a & 255\n").unwrap();
    assert_eq!(
        check("f(a in 0 to 255, b in 0 to 255) in 0 to 254: return a | b\n"),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn shifts_multiply_and_divide_by_powers_of_two() {
    check("f(a in 0 to 100, n in 0 to 3) in 0 to 800: return a << n\n").unwrap();
    check("f(a in -100 to 100, n in 1 to 2) in -50 to 50: return a >> n\n").unwrap();
    check("f(a in 5 to 5, n in 2 to 2) in 20 to 20: return a << n\n").unwrap();
    check("f(a in -5 to -5, n in 1 to 1) in -3 to -3: return a >> n\n").unwrap();
    assert_eq!(
        check("f(a in 0 to 100, n in 0 to 3) in 0 to 799: return a << n\n"),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_shift_by_a_negative_or_by_too_much_is_not_proved() {
    assert_eq!(
        check("f(a in 0 to 100, n in -1 to 3) in 0 to 800: return a << n\n"),
        Err(SourceCheckError::Unproved)
    );
    assert_eq!(
        check("f(a in 0 to 100, n in 0 to 300) in 0 to 800: return a >> n\n"),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_guard_with_or_proves_a_shift_in_a_conditional() {
    check(
        "f(a in 0 to 100, n in -50 to 50) in -1 to 100:\n    return when n < 0 or n >= 8 then -1 else a >> n\n",
    )
    .unwrap();
    // Without the upper cut the shift can still be too wide.
    check(
        "f(a in 0 to 100, n in -50 to 300) in -1 to 100:\n    return when n < 0 or n >= 256 then -1 else a >> n\n",
    )
    .unwrap();
    assert_eq!(
        check(
            "f(a in 0 to 100, n in -50 to 300) in -1 to 100:\n    return when n < 0 then -1 else a >> n\n"
        ),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_while_condition_may_combine_comparisons() {
    // `or`: the loop runs while either holds and ends when both have failed.
    check(
        "f(value in -1000 to 1000) in 0 to 9:\n    number in -1000 to 1000: value\n    while number <= -10 or number >= 10 do\n        number :: number / 10\n    return when number < 0 then 0 - number else number\n",
    )
    .unwrap();
    // `and`: the loop runs while both hold.
    check(
        "f() in 0 to 10:\n    i in 0 to 10: 0\n    j in 0 to 10: 0\n    while i < 10 and j < 10 do\n        i :: i + 1\n    return i\n",
    )
    .unwrap();
    // After an `or` loop both comparisons are false, so this result is too tight for -9.
    assert_eq!(
        check(
            "f(value in -1000 to 1000) in 0 to 9:\n    number in -1000 to 1000: value\n    while number <= -10 or number >= 10 do\n        number :: number / 10\n    return number\n"
        ),
        Err(SourceCheckError::Type)
    );
}
