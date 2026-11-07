//! An unannotated parameter is a type parameter: `identity(element, value element) element`.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const PAIR: &str = "Pair(element) has left element, right element\n\n";

#[test]
fn a_bare_name_declares_a_type_parameter() {
    check(&format!(
        "{PAIR}make(element, a element, b element) Pair(element): Pair(left: a, right: b)\n"
    ))
    .unwrap();
}

#[test]
fn it_means_the_same_as_the_explicit_kind() {
    let bare = format!("{PAIR}swap(element, p Pair(element)) Pair(element):\n    Pair(left, right) := p\n    Pair(left: right, right: left)\n");
    let explicit = bare.replace("swap(element,", "swap(element Type,");
    check(&bare).unwrap();
    check(&explicit).unwrap();
}

#[test]
fn a_type_parameter_is_still_linear_so_it_cannot_be_dropped() {
    assert_eq!(
        check(&format!(
            "{PAIR}drop(element, p Pair(element)):\n    Pair(left, right) := p\n    left\n"
        )),
        Err(SourceCheckError::Unconsumed)
    );
}
