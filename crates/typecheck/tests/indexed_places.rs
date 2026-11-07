//! An indexed place parses; writing one element is a checked place write (`element_writes.rs`).
use typecheck::{Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.places", &source, &[], Limits::default()).map(|_| ())
}

#[test]
fn writing_an_element_is_a_checked_place_write() {
    let source = "#unrestricted\nDigit is in 0 to 9\n#unrestricted\nSmall is in 0 to 3\n\nf(cells mut [Digit; 3], n Digit) in 0 to 9:\n    cells.[0] :: n\n    return 0\n";
    assert_eq!(check(source), Ok(()));
    // The elements are typed by the array's type: a `Digit` is not a `Small`.
    let wide = "#unrestricted\nDigit is in 0 to 9\n#unrestricted\nSmall is in 0 to 3\n\nf(cells mut [Small; 3], n Digit) in 0 to 9:\n    cells.[0] :: n\n    return 0\n";
    assert_eq!(check(wide), Err(SourceCheckError::Type));
}

/// A field path is still a place, and still works.
#[test]
fn a_field_path_is_unchanged() {
    let source = "#unrestricted\nBox has\n    value in 0 to 9\n\nf() in 0 to 9:\n    held: Box(value: 1)\n    held.value :: 4\n    return held.value\n";
    assert_eq!(check(source), Ok(()));
}
