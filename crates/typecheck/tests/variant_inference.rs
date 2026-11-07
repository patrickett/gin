//! A bare variant where the expected type is only inferred (a generic record's field), and the
//! qualified form on a generic union.
use typecheck::{Limits, SourceCheckError, check_file_diagnostic};

fn run(text: &str) -> Result<(), typecheck::SourceCheckFailure> {
    let file = parser::parse_strict(text).unwrap_or_else(|d| panic!("{text}\n{d:?}"));
    check_file_diagnostic(&file, Limits::default()).map(|_| ())
}

const SLOT: &str = "#unrestricted\nScalar is in 0 to 9\n\nSlot(element) is Empty or Full(element)\n\nBuffer(element) has\n    target Slot(element)\n    rest Slot(element)\n\n";

fn accepts(tail: &str) {
    let text = format!("{SLOT}{tail}");
    if let Err(failure) = run(&text) {
        panic!("{text}\n{failure:?}");
    }
}

#[test]
fn a_bare_variant_in_a_generic_record_constructor_infers_the_type_argument() {
    accepts("make(n Scalar) Buffer(Scalar): return Buffer(target: Full(n), rest: Empty)\n");
    accepts("make(n Scalar) Buffer(Scalar):\n    b: Buffer(target: Full(n), rest: Empty)\n    return b\n");
}

#[test]
fn a_qualified_variant_of_a_generic_union_infers_or_explains() {
    accepts("make(n Scalar) Slot(Scalar): return Slot.Full(n)\n");
    accepts("make(n Scalar) Slot(Scalar): return Slot.Empty\n");
    // Negative control: the payload of the wrong type is a `Type` error with a reason.
    let text = format!("{SLOT}Other has v in 0 to 1\nmake(n Scalar) Slot(Scalar): return Slot.Full(Other(v: 1))\n");
    let failure = run(&text).unwrap_err();
    assert_eq!(failure.error, SourceCheckError::Type);
    // With no expected type there is nothing to read the instance from: said in words.
    let text = format!("{SLOT}make(n Scalar):\n    s: Slot.Full(n)\n    return\n");
    let failure = run(&text).unwrap_err();
    assert!(failure.detail.is_some(), "{failure:?}");
}

#[test]
fn a_bare_variant_with_nothing_to_infer_it_from_is_explained() {
    let text = format!("{SLOT}make():\n    s: Empty\n    return\n");
    let failure = run(&text).unwrap_err();
    assert!(failure.detail.is_some(), "{failure:?}");
}
