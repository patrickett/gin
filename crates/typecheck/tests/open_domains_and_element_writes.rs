//! An `#unrestricted` number-like domain is a plain number with a name: its number is readable
//! everywhere (arithmetic, `x in Off: field`, array indexes, `where` clause values), while a
//! linear domain keeps its number to its own methods. And `array.[i] :: value` is a checked
//! place write: index proved in bounds, the array held exclusively, the old element droppable.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const OFF: &str = "\
cap := 1024
#unrestricted
Off is in 0 to cap

";

#[test]
fn an_open_domain_takes_part_in_arithmetic() {
    check(&format!("{OFF}next(a Off and a < cap) Off: return (a + 1) as Off\n")).unwrap();
    // The cast is still proved: without the claim the sum can leave the range.
    assert_eq!(
        check(&format!("{OFF}next(a Off) Off: return (a + 1) as Off\n")),
        Err(SourceCheckError::Unproved)
    );
    check(&format!("{OFF}twice(a Off and a <= 512) Off: return (a * 2) as Off\n")).unwrap();
}

#[test]
fn an_open_domain_is_read_as_a_number_anywhere() {
    check(&format!("{OFF}read(a Off and a < 10) in 0 to 9:\n    n in Off: a\n    return n\n")).unwrap();
}

#[test]
fn a_linear_domain_keeps_its_number_private() {
    let linear = "Token is in 0 to 9\nToken has\n    release(self): return\n\n";
    assert!(check(&format!("{linear}read(a Token) in 0 to 9:\n    n in Token: a\n    return n\n")).is_err());
    assert!(check(&format!("{linear}next(a Token) Token: return (a + 1) as Token\n")).is_err());
}

#[test]
fn a_where_clause_reads_a_domain_field_as_its_number() {
    let record = format!(
        "{OFF}Buf has\n    length Off\n\n    release(self):\n        Buf(length) := self\n        return\n\n"
    );
    let body = "        buffer.length :: buffer.length + 1\n        return\n";
    check(&format!(
        "{record}    grow(buffer mut Self and buffer.length < cap) where buffer.length :: buffer.length + 1:\n{body}"
    ))
    .unwrap();
    // The clause says one more; the body adds two.
    assert_eq!(
        check(&format!(
            "{record}    grow(buffer mut Self and buffer.length < cap) where buffer.length :: buffer.length + 1:\n        buffer.length :: buffer.length + 2\n        return\n"
        )),
        Err(SourceCheckError::Unproved)
    );
    // A caller sees the effect: two calls make two more, and need room for both.
    check(&format!(
        "{record}    grow(buffer mut Self and buffer.length < cap) where buffer.length :: buffer.length + 1:\n{body}\n    twice(buffer mut Self and buffer.length <= cap - 2) where buffer.length :: buffer.length + 2:\n        Buf.grow(buffer)\n        Buf.grow(buffer)\n        return\n"
    ))
    .unwrap();
    assert_eq!(
        check(&format!(
            "{record}    grow(buffer mut Self and buffer.length < cap) where buffer.length :: buffer.length + 1:\n{body}\n    twice(buffer mut Self and buffer.length <= cap - 1):\n        Buf.grow(buffer)\n        Buf.grow(buffer)\n        return\n"
        )),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_claim_may_combine_literals_and_constants() {
    check("cap := 1024\nf(n in 0 to 2000 and n <= cap - 4) in 0 to 1020: return n\n").unwrap();
    assert_eq!(
        check("cap := 1024\nf(n in 0 to 2000 and n <= cap - 4) in 0 to 1019: return n\n"),
        Err(SourceCheckError::Type)
    );
}

const BYTES: &str = "\
#unrestricted
Byte is in 0 to 255

";

#[test]
fn a_known_slot_of_an_array_is_written() {
    check(&format!("{BYTES}put(a mut [Byte; 4]):\n    a.[2] :: 7\n    return\n")).unwrap();
    // Out of bounds is not a slot.
    assert_eq!(
        check(&format!("{BYTES}put(a mut [Byte; 4]):\n    a.[4] :: 7\n    return\n")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_computed_index_must_be_proved_in_bounds() {
    check(&format!("{BYTES}put(a mut [Byte; 4], i in 0 to 3):\n    a.[i] :: 7\n    return\n")).unwrap();
    assert!(check(&format!("{BYTES}put(a mut [Byte; 4], i in 0 to 4):\n    a.[i] :: 7\n    return\n")).is_err());
    // An index that is an open domain is its number.
    check(&format!(
        "{BYTES}#unrestricted\nSlot is in 0 to 3\n\nput(a mut [Byte; 4], i Slot):\n    a.[i] :: 7\n    return\n"
    ))
    .unwrap();
}

#[test]
fn the_value_is_viewed_as_the_element_type() {
    assert!(check(&format!("{BYTES}put(a mut [Byte; 4], i in 0 to 3):\n    a.[i] :: 300\n    return\n")).is_err());
}

#[test]
fn the_array_must_be_held_exclusively() {
    assert!(check(&format!("{BYTES}put(a ref [Byte; 4], i in 0 to 3):\n    a.[i] :: 7\n    return\n")).is_err());
}

#[test]
fn a_linear_element_is_not_dropped_by_a_write() {
    let linear = "Token has\n    number in 0 to 3\n\n    release(self):\n        Token(number) := self\n        return\n\n";
    assert_eq!(
        check(&format!("{linear}put(a mut [Token; 4], i in 0 to 3, t Token):\n    a.[i] :: t\n    return\n")),
        Err(SourceCheckError::OverwriteLinear)
    );
}

#[test]
fn a_write_through_a_mut_view_of_a_field_leaves_other_fields_alone() {
    let record = format!(
        "{BYTES}#unrestricted\nOff is in 0 to 1024\n\nBuf has\n    bytes [Byte; 8]\n    length Off\n\n    release(self):\n        Buf(bytes, length) := self\n        for item in bytes do\n            ()\n        return\n\n"
    );
    check(&format!(
        "{record}    put(buffer mut Self, value Byte):\n        storage: mut buffer.bytes\n        storage.[3] :: value\n        buffer.length :: 5\n        return\n"
    ))
    .unwrap();
}
