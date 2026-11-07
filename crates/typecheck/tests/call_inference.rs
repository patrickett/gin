//! A call leaves out the type and number arguments its arguments or its expected type
//! determine; what neither determines must be written.
use typecheck::{Limits, SourceCheckError, check_file, check_file_diagnostic};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

fn detail(text: &str) -> String {
    let source = parser::parse_strict(text).expect("strict source syntax");
    let failure = check_file_diagnostic(&source, Limits::default()).expect_err(text);
    failure.detail.unwrap_or_default()
}

const METER: &str = "\
Meter(capacity Length) has
    used Length

    start() Self: return Self(used: 0)

";

const VIEW: &str = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 1000

View(element, capacity Length) has
    len Length
    storage ref [element; len]

    new(storage ref [element; capacity], len Length and len <= capacity) Self:
        region: ref storage.[0; len]
        return Self(len: len, storage: region)

";

fn with(tail: &str) -> String {
    format!("{VIEW}{METER}{tail}\n")
}

#[test]
fn type_and_number_come_from_the_array_argument() {
    check(&with(
        "make(storage ref [Byte; 16]) View(Byte, 16):\n    return View.new(storage, 4)",
    ))
    .unwrap();
}

#[test]
fn the_inferred_instance_is_the_one_the_arguments_give() {
    // Not a different element ...
    assert!(
        check(&with(
            "make(storage ref [Byte; 16]) View(Length, 16):\n    return View.new(storage, 4)"
        ))
        .is_err()
    );
    // ... and not a different number.
    assert!(
        check(&with(
            "make(storage ref [Byte; 16]) View(Byte, 8):\n    return View.new(storage, 4)"
        ))
        .is_err()
    );
}

#[test]
fn the_claim_is_checked_against_the_inferred_number() {
    assert_eq!(
        check(&with(
            "make(storage ref [Byte; 16]) View(Byte, 16):\n    return View.new(storage, 17)"
        )),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn the_expected_type_fills_what_the_arguments_leave_open() {
    check(&with(
        "make() Meter(16):\n    return Meter.start()",
    ))
    .unwrap();
    check(&with(
        "make():\n    m Meter(16): Meter.start()\n    Meter(used) := m\n    return",
    ))
    .unwrap();
}

#[test]
fn an_open_parameter_with_no_expected_type_is_named() {
    let text = with("make():\n    m := Meter.start()\n    Meter(used) := m\n    return");
    let message = detail(&text);
    assert!(message.contains("`capacity`"), "{message}");
}

#[test]
fn only_the_unresolved_parameter_is_named() {
    // `element` is inferred from the argument; `capacity` is what is left.
    let source = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 1000

Tray(element, capacity Length) has
    item element

    make(item element) Self: return Self(item: item)

use_it(b Byte):
    t := Tray.make(b)
    return
";
    let message = detail(source);
    assert!(message.contains("`capacity`"), "{message}");
    assert!(!message.contains("`element`"), "{message}");
}

#[test]
fn conflicting_arguments_are_an_error() {
    let source = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 1000

Tray(element) has
    left element
    right element

    make(left element, right element) Self: return Self(left: left, right: right)

use_it(a Byte, b Length) Tray(Byte):
    return Tray.make(a, b)
";
    assert_eq!(check(source), Err(SourceCheckError::Type));
    let message = detail(source);
    assert!(message.contains("argument 2"), "{message}");
}

#[test]
fn the_callers_own_generics_are_arguments() {
    check(&with(
        "pass(element, storage ref [element; 16]) View(element, 16):\n    return View.new(storage, 4)",
    ))
    .unwrap();
}

#[test]
fn the_callers_own_number_is_an_argument() {
    // Inside a method of the same type, `capacity` is the caller's abstract number.
    let source = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 1000

Cell(element, capacity Length) has
    count Length

    over(storage ref [element; capacity]) Self: return Self(count: 0)

    again(storage ref [element; capacity]) Self:
        return Cell.over(storage)
";
    check(source).unwrap();
    // The abstract number is that number, not another.
    assert!(check(&source.replace("Self:\n        return Cell.over", "Cell(element, 8):\n        return Cell.over")).is_err());
}

const BOUND: &str = "\
Release has
    release(self)

#unrestricted
Good is in 0 to 9
Good has Release
    release(self): return

#unrestricted
Bad is in 0 to 9

Wrapped(element has Release) has
    value element

    around(value element) Self: return Self(value: value)

";

#[test]
fn the_bound_is_checked_after_inference() {
    check(&format!("{BOUND}hold(v Good) Wrapped(Good): Wrapped.around(v)\n")).unwrap();
    assert_eq!(
        check(&format!("{BOUND}hold(v Bad) Wrapped(Bad): Wrapped.around(v)\n")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_number_outside_the_declared_range_is_refused() {
    let source = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 10

Small(element, capacity Length) has
    count Length

    over(storage ref [element; capacity]) Self: return Self(count: 0)

use_it(storage ref [Byte; 20]):
    s := Small.over(storage)
    return
";
    assert!(check(source).is_err());
}

#[test]
fn an_explicit_application_names_every_argument() {
    check(&with(
        "make(storage ref [Byte; 16]) View(Byte, 16):\n    return View(Byte, 16).new(storage, 4)",
    ))
    .unwrap();
    // The written type is checked against the arguments.
    assert!(
        check(&with(
            "make(storage ref [Byte; 16]) View(Byte, 8):\n    return View(Byte, 8).new(storage, 4)"
        ))
        .is_err()
    );
    // It also supplies what nothing else could.
    check(&with(
        "make():\n    m := Meter(16).start()\n    Meter(used) := m\n    return",
    ))
    .unwrap();
}

const FREE: &str = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 1000

Pair(element) has left element, right element
Slot(element) is Empty or Full(element)

identity(element, value element) element: value

both(element, left element, right element) Pair(element):
    return Pair(left: left, right: right)

nothing(element) Slot(element): Empty

";

#[test]
fn the_explicit_free_function_style_is_unchanged() {
    check(&format!("{FREE}use_it(b Byte) Byte: identity(Byte, b)\n")).unwrap();
    assert!(check(&format!("{FREE}use_it(b Byte) Length: identity(Length, b)\n")).is_err());
}

#[test]
fn a_free_function_may_omit_its_leading_type_argument() {
    check(&format!("{FREE}use_it(b Byte) Byte: identity(b)\n")).unwrap();
    check(&format!("{FREE}use_it(a Byte, b Byte) Pair(Byte): both(a, b)\n")).unwrap();
}

#[test]
fn a_free_function_takes_the_omitted_type_from_the_expected_type() {
    check(&format!(
        "{FREE}use_it() Slot(Byte): nothing()\n"
    ))
    .unwrap();
}

#[test]
fn an_omitted_type_nothing_determines_is_named() {
    let message = detail(&format!("{FREE}use_it():\n    p := nothing()\n    return\n"));
    assert!(message.contains("`element`"), "{message}");
}

#[test]
fn an_inferred_free_call_has_the_inferred_type() {
    assert!(check(&format!("{FREE}use_it(b Byte) Length: identity(b)\n")).is_err());
}

#[test]
fn the_inferred_type_may_be_the_callers_generic() {
    check(&format!(
        "{FREE}forward(element, value element) element: identity(value)\n"
    ))
    .unwrap();
}

#[test]
fn too_many_arguments_stay_an_error() {
    assert!(check(&format!("{FREE}use_it(b Byte) Byte: identity(Byte, b, b)\n")).is_err());
}
