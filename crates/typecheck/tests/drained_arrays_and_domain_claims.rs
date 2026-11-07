//! Four things the draft `core` package needed from the checker:
//! * a loop that ends every element of an owned array (an array has no methods, so a library
//!   record around one writes its own `release`);
//! * `in Byte` and `in domain of Byte`: the range a declared domain holds, where a range goes;
//! * a claim on a domain-typed parameter (`previous Off and previous < cap`) is a fact about the
//!   number it holds, and a `where` claim may read a field of the named result;
//! * a bound on a type parameter is checked against a type of this module even when another
//!   type's field names it before its layout is compiled.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const DRAIN: &str = "\
Release has
    release(self)

#unrestricted
Byte is in 0 to 255
Byte has Release
    release(self): return

Token has Release
    number in 0 to 3

    release(self):
        Token(number) := self
        return

";

#[test]
fn a_loop_over_an_owned_array_ends_every_element() {
    check(&format!(
        "{DRAIN}Store(element has Release, count in 1 to 100) has\n    items [element; count]\n\n    release(self):\n        Store(items) := self\n        for item in items do\n            item.release\n        return\n"
    ))
    .unwrap();
}

#[test]
fn the_body_must_end_the_element_it_is_given() {
    assert_eq!(
        check(&format!(
            "{DRAIN}Store(element has Release, count in 1 to 100) has\n    items [element; count]\n\n    release(self):\n        Store(items) := self\n        for item in items do\n            ()\n        return\n"
        )),
        Err(SourceCheckError::Unconsumed)
    );
}

#[test]
fn a_borrowed_array_cannot_be_drained() {
    assert_eq!(
        check(&format!(
            "{DRAIN}drain(element has Release, items ref [element; 4]):\n    for item in items do\n        item.release\n    return\n"
        )),
        Err(SourceCheckError::MoveOutOfBorrow)
    );
}

#[test]
fn a_bound_names_a_type_declared_in_the_same_module() {
    // `Outer.held` names `Byte` before any layout is compiled, and `Byte` provides `Release`.
    check(&format!(
        "{DRAIN}Holder(element has Release) has\n    first element\n\n    release(self):\n        Holder(first) := self\n        element.release(first)\n        return\n\nOuter has\n    held Holder(Byte)\n\n    release(self):\n        Outer(held) := self\n        held.release\n        return\n"
    ))
    .unwrap();
    // A type that does not provide it is still refused.
    assert_eq!(
        check(&format!(
            "{DRAIN}#unrestricted\nPlain is in 0 to 3\n\nHolder(element has Release) has\n    first element\n\n    release(self):\n        Holder(first) := self\n        element.release(first)\n        return\n\nOuter has\n    held Holder(Plain)\n\n    release(self):\n        Outer(held) := self\n        held.release\n        return\n"
        )),
        Err(SourceCheckError::Type)
    );
}

const OFFSET: &str = "\
cap := 1024
#unrestricted
Off is in 0 to cap

";

#[test]
fn a_claim_on_a_domain_parameter_bounds_the_number_it_holds() {
    let advance = "Off has\n    advance(self Self and self < cap) Off:\n        return ((position in Self: self) + 1) as Self\n";
    check(&format!("{OFFSET}{advance}")).unwrap();
    // Without the claim the successor of the largest offset is out of range.
    assert_eq!(
        check(&format!(
            "{OFFSET}Off has\n    advance(self) Off:\n        return ((position in Self: self) + 1) as Self\n"
        )),
        Err(SourceCheckError::Unproved)
    );
    // A caller must show the claim ...
    check(&format!("{OFFSET}{advance}\nstep(at Off and at < cap) Off: return Off.advance(at)\n")).unwrap();
    assert_eq!(
        check(&format!("{OFFSET}{advance}\nstep(at Off) Off: return Off.advance(at)\n")),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_domain_name_is_the_range_it_holds_where_a_range_goes() {
    check("#unrestricted\nByte is in 0 to 255\n\nf(n in 0 to 255) Byte:\n    m in Byte: n\n    return m as Byte\n").unwrap();
    check("#unrestricted\nByte is in 0 to 255\n\nf(n in 0 to 255) Byte:\n    m in domain of Byte: n\n    return m as Byte\n").unwrap();
    // The number must fit the range the name stands for.
    assert!(check("#unrestricted\nByte is in 0 to 255\n\nf(n in 0 to 300) Byte:\n    m in Byte: n\n    return m as Byte\n").is_err());
}

#[test]
fn a_where_claim_reads_a_field_of_the_named_result() {
    let record = "#unrestricted\nOff is in 0 to 1024\n\nBuffer has\n    length Off\n\n";
    check(&format!(
        "{record}    new() buffer Self where buffer.length = 0: return Self(length: 0)\n"
    ))
    .unwrap();
    assert_eq!(
        check(&format!(
            "{record}    new() buffer Self where buffer.length = 1: return Self(length: 0)\n"
        )),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_claim_may_be_about_a_field_of_a_parameter() {
    let head = "cap := 1024\n#unrestricted\nOff is in 0 to cap\n\nOff has\n    advance(self Self and self < cap) Off:\n        return ((next in Self: self) + 1) as Self\n\nBuf has\n    length Off\n\n    release(self):\n        Buf(length) := self\n        return\n\n";
    // The claim is assumed inside the method and shown at each call.
    check(&format!(
        "{head}    grow(buffer mut Self and buffer.length < cap):\n        buffer.length :: Off.advance(buffer.length)\n        return\n"
    ))
    .unwrap();
    assert_eq!(
        check(&format!(
            "{head}    grow(buffer mut Self):\n        buffer.length :: Off.advance(buffer.length)\n        return\n"
        )),
        Err(SourceCheckError::Unproved)
    );
    let grow = "    grow(buffer mut Self and buffer.length < cap):\n        return\n\n";
    check(&format!(
        "{head}{grow}caller(buffer mut Buf and buffer.length < cap):\n    Buf.grow(buffer)\n    return\n"
    ))
    .unwrap();
    assert_eq!(
        check(&format!("{head}{grow}caller(buffer mut Buf):\n    Buf.grow(buffer)\n    return\n")),
        Err(SourceCheckError::Unproved)
    );
}
