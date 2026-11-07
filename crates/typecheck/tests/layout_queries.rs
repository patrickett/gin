//! `#size(T)` and `#alignment(T)` are numbers known at check time: a domain takes the fewest whole
//! machine bytes its range needs, a record places its fields in order on their alignments.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const TYPES: &str = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Short is in 0 to 65535
#unrestricted
Word is in -2147483648 to 2147483647
#unrestricted
Wide is in 0 to 18446744073709551615
#unrestricted
Mixed has
    small Byte
    wide Wide
    short Short

";

fn with(tail: &str) -> String {
    format!("{TYPES}{tail}\n")
}

#[test]
fn a_domain_takes_the_bytes_its_range_needs() {
    check(&with("a() in 1 to 1: return #size(Byte)")).unwrap();
    check(&with("b() in 2 to 2: return #size(Short)")).unwrap();
    check(&with("c() in 4 to 4: return #size(Word)")).unwrap();
    check(&with("d() in 8 to 8: return #size(Wide)")).unwrap();
}

#[test]
fn a_domain_is_aligned_to_its_size() {
    check(&with("a() in 4 to 4: return #alignment(Word)")).unwrap();
    check(&with("b() in 1 to 1: return #alignment(Byte)")).unwrap();
}

#[test]
fn a_record_places_its_fields_and_rounds_up() {
    // Byte at 0, Wide at 8, Short at 16, rounded up to a multiple of 8.
    check(&with("a() in 24 to 24: return #size(Mixed)")).unwrap();
    check(&with("b() in 8 to 8: return #alignment(Mixed)")).unwrap();
}

#[test]
fn the_number_is_exact() {
    assert_eq!(
        check(&with("a() in 2 to 2: return #size(Byte)")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_pointer_is_a_register() {
    check(&with("a() in 8 to 8: return #size(@Byte)")).unwrap();
}

#[test]
fn a_type_parameter_is_some_positive_number() {
    check(&with(
        "Cell(element) has\n    value element\n\n    width() in 1 to 4611686018427387904: return #size(element)\n",
    ))
    .unwrap();
    // ... and not one in particular.
    assert_eq!(
        check(&with(
            "Cell(element) has\n    value element\n\n    width() in 8 to 8: return #size(element)\n"
        )),
        Err(SourceCheckError::Type)
    );
}
