//! Collection forms outside the fixed-array checkpoint must still fail closed.
use typecheck::{Limits, SourceCheckError, check_file};

#[test]
fn remaining_collection_forms_stay_explicitly_unsupported() {
    for text in [
        "make: return (0; 2)\n",
        "first: return [1, 2].[0]\n",
        "slice: return [1, 2].[0; 1]\n",
        "bounds: return 0 to 1\n",
    ] {
        let source = parser::parse_strict(text).expect("supported source syntax");
        let before = source.clone();
        assert_eq!(
            check_file(&source, Limits::default()),
            Err(SourceCheckError::Unsupported),
            "{text}"
        );
        assert_eq!(source, before);
    }
}

#[test]
fn an_array_result_is_not_a_unit_result() {
    let source = parser::parse_strict("make: return [1, 2]\n").unwrap();
    assert_eq!(
        check_file(&source, Limits::default()),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_while_loop_that_never_ends_stops_the_path_and_a_finite_range_loop_is_checked() {
    // A range loop is checked once for any number of passes (see `range_loops.rs`).
    let range = parser::parse_strict(
        "scan:\n    for index in 0 to 1 do\n        index\n    return\n",
    )
    .expect("supported loop syntax");
    assert!(check_file(&range, Limits::default()).is_ok());
    // A `while` whose condition always holds never ends: the path stops there, as at a `return`.
    let unbounded =
        parser::parse_strict("scan:\n    while 0 < 1 do\n        0\n    return\n")
            .expect("supported loop syntax");
    assert!(check_file(&unbounded, Limits::default()).is_ok());
    // A function that gives a number is fine too: what a loop that does not end gives is never seen.
    let result =
        parser::parse_strict("scan() in 0 to 1:\n    while 0 < 1 do\n        0\n    return 0\n")
            .expect("supported loop syntax");
    assert!(check_file(&result, Limits::default()).is_ok());
    // Negative control: a loop that can end still needs what follows it to give the result.
    let ends = parser::parse_strict("scan(n in 0 to 3) in 0 to 1:\n    while n < 2 do\n        0\n    return 5\n")
        .expect("supported loop syntax");
    assert!(check_file(&ends, Limits::default()).is_err());
}

#[test]
fn sequence_annotations_do_not_guess_a_fixed_extent() {
    let source = parser::parse_strict("Holder(element) has values [element]\n").unwrap();
    assert_eq!(
        check_file(&source, Limits::default()),
        Err(SourceCheckError::Unsupported)
    );
}
