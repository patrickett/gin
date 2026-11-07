//! A type parameter annotated by an integer domain (`capacity Length`) takes a number known at
//! check time. Instances with different numbers are different types; the number is also a value
//! inside the type's methods and the length of `[element; capacity]`.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const BASE: &str = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 1000
size() in 16 to 16: return 16

Buf(element, capacity Length) has
    len in 0 to 1000

    room(self ref) Length: return capacity
    fits(self ref, n in 0 to 1000 and n <= capacity) Length: return n

";

fn with(tail: &str) -> String {
    format!("{BASE}{tail}\n")
}

#[test]
fn a_number_argument_applies_the_type() {
    check(&with("make() Buf(Byte, 16): Buf(len: 0)")).unwrap();
    check(&with("same(b Buf(Byte, 16)) Buf(Byte, 16): b")).unwrap();
}

#[test]
fn a_constant_is_a_number_argument() {
    check(&with("named(b Buf(Byte, size)) Buf(Byte, 16): b")).unwrap();
    // ... and it is that number, not another.
    assert_eq!(
        check(&with("named(b Buf(Byte, size)) Buf(Byte, 8): b")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn instances_with_different_numbers_are_different_types() {
    assert_eq!(
        check(&with("wrong(b Buf(Byte, 16)) Buf(Byte, 8): b")),
        Err(SourceCheckError::Type)
    );
    // The element still counts.
    assert_eq!(
        check(&with("wrong(b Buf(Byte, 16)) Buf(Length, 16): b")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_number_outside_the_parameters_range_is_refused() {
    assert_eq!(
        check(&with("big(b Buf(Byte, 2000)) Length: return 0")),
        Err(SourceCheckError::Type)
    );
    assert_eq!(
        check(&with("small(b Buf(Byte, -1)) Length: return 0")),
        Err(SourceCheckError::Type)
    );
    // The ends of the range are inside it.
    check(&with("edge(b ref Buf(Byte, 1000)) Length: return 0")).unwrap();
    check(&with("edge(b ref Buf(Byte, 0)) Length: return 0")).unwrap();
}

#[test]
fn a_number_is_not_a_type_and_a_type_is_not_a_number() {
    assert_eq!(
        check(&with("a(b Buf(Byte, Byte)) Length: return 0")),
        Err(SourceCheckError::Type)
    );
    assert_eq!(
        check(&with("b(b Buf(16, 16)) Length: return 0")),
        Err(SourceCheckError::Type)
    );
    // A number must be given: it has no default.
    assert!(check(&with("c(b Buf(Byte)) Length: return 0")).is_err());
}

#[test]
fn the_number_is_a_value_in_the_types_methods() {
    // It ranges over what its annotation allows, so this is proved ...
    check(&with("")).unwrap();
    // ... and a bound tighter than that range is not.
    let tight = BASE.replace(
        "room(self ref) Length: return capacity",
        "room(self ref) in 0 to 10: return capacity",
    );
    assert_eq!(check(&tight), Err(SourceCheckError::Type));
}

#[test]
fn a_claim_over_the_number_is_checked_where_the_instance_is_known() {
    check(&with("ok(b ref Buf(Byte, 16)) Length: return b.fits(10)")).unwrap();
    check(&with("edge(b ref Buf(Byte, 16)) Length: return b.fits(16)")).unwrap();
    assert_eq!(
        check(&with("over(b ref Buf(Byte, 16)) Length: return b.fits(17)")),
        Err(SourceCheckError::Unproved)
    );
    // The same call on a larger instance is fine.
    check(&with("big(b ref Buf(Byte, 32)) Length: return b.fits(17)")).unwrap();
}

const VIEW: &str = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 1000

View(element, capacity Length) has
    len in 0 to 1000
    storage ref [element; len]

";

#[test]
fn an_array_of_capacity_elements_bounds_a_slice_of_it() {
    // `len <= capacity` is what lets `storage.[0; len]` be proved inside the array.
    check(&format!(
        "{VIEW}    new(storage ref [element; capacity], len in 0 to 1000 and len <= capacity) Self:\n        region: ref storage.[0; len]\n        return Self(len: len, storage: region)\n"
    ))
    .unwrap();
}

#[test]
fn without_the_claim_the_slice_is_not_proved() {
    assert_eq!(
        check(&format!(
            "{VIEW}    new(storage ref [element; capacity], len in 0 to 1000) Self:\n        region: ref storage.[0; len]\n        return Self(len: len, storage: region)\n"
        )),
        Err(SourceCheckError::Unproved)
    );
    // A claim that is too weak does not prove it either.
    assert_eq!(
        check(&format!(
            "{VIEW}    new(storage ref [element; capacity], len in 0 to 1000 and len <= 1000) Self:\n        region: ref storage.[0; len]\n        return Self(len: len, storage: region)\n"
        )),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn an_unnamed_number_is_not_a_parameter() {
    assert!(check(&format!(
        "{VIEW}    new(storage ref [element; other], len in 0 to 1000) Self:\n        region: ref storage.[0; len]\n        return Self(len: len, storage: region)\n"
    ))
    .is_err());
}

const WINDOW: &str = "\
Win has
    len in 0 to 9

    at(self ref, n in 0 to 9 and n < self.len) in 0 to 8: return n

";

#[test]
fn a_claim_names_a_field_of_a_parameter_by_path() {
    // The body may rely on `n < self.len`, so `n` is at most 8.
    check(&format!("{WINDOW}")).unwrap();
    // The caller must prove it.
    check(&format!(
        "{WINDOW}ok(w ref Win) in 0 to 8:\n    when w.len > 0 then return w.at(0)\n    return 0\n"
    ))
    .unwrap();
    assert_eq!(
        check(&format!("{WINDOW}bad(w ref Win) in 0 to 8: return w.at(0)\n")),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_path_claim_needs_a_field_that_exists() {
    let source = WINDOW.replace("n < self.len", "n < self.missing");
    assert!(check(&source).is_err());
}

#[test]
fn the_length_may_be_held_in_a_named_domain() {
    // `len Length`: the array is as long as the number the domain holds.
    let source = "\
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

    narrow(self ref) View(element, capacity):
        return View(len: self.len, storage: ref self.storage)
";
    check(source).unwrap();
    // The claim is still what proves the slice.
    assert_eq!(
        check(&source.replace(" and len <= capacity", "")),
        Err(SourceCheckError::Unproved)
    );
}

const FORWARD: &str = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 1000

View(element, capacity Length) has
    len in 0 to 1000
    storage ref [element; len]

    new(storage ref [element; capacity], len in 0 to 1000 and len <= capacity) Self:
        region: ref storage.[0; len]
        return Self(len: len, storage: region)

";

#[test]
fn a_callers_abstract_number_proves_the_callees_claim() {
    // The caller claims `n <= capacity`, which is the callee's `len <= capacity` for its own
    // `capacity`.
    check(&format!(
        "{FORWARD}    again(self ref, storage ref [element; capacity], n in 0 to 1000 and n <= capacity) Self:\n        return View.new(storage, n)\n"
    ))
    .unwrap();
    // Without it the claim is not proved.
    assert_eq!(
        check(&format!(
            "{FORWARD}    again(self ref, storage ref [element; capacity], n in 0 to 1000) Self:\n        return View.new(storage, n)\n"
        )),
        Err(SourceCheckError::Unproved)
    );
    // A claim about some other bound is no help.
    assert_eq!(
        check(&format!(
            "{FORWARD}    again(self ref, storage ref [element; capacity], n in 0 to 1000 and n <= 1000) Self:\n        return View.new(storage, n)\n"
        )),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_free_function_may_take_a_number_parameter() {
    let base = "#unrestricted\nByte is in 0 to 255\n#unrestricted\nLength is in 0 to 1000\n\nfirst(element, capacity Length, storage ref [element; capacity]) Length: return capacity\n\n";
    // Written ...
    check(&format!("{base}use_it(s ref [Byte; 16]) Length: return first(Byte, 16, s)\n")).unwrap();
    // ... or read off the array.
    check(&format!("{base}use_it(s ref [Byte; 16]) Length: return first(s)\n")).unwrap();
    // It is the array's number, not another.
    assert!(check(&format!("{base}use_it(s ref [Byte; 16]) Length: return first(Byte, 8, s)\n")).is_err());
}
