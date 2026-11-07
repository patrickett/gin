//! A range is a value: `domain of T` and `a to b` are accepted where a type argument goes, and
//! `#lower(r)` / `#upper(r)` read its ends at check time. The compiler has no `.min` or `.max` of
//! its own for a library type: `Range(r)` is ordinary library code built on these two queries.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const RANGE: &str = "\
Range(r) has
    value r

    min() r: return #lower(r)

    max() r: return #upper(r)

#unrestricted
Byte is in 0 to 255

";

fn with(tail: &str) -> String {
    format!("{RANGE}{tail}\n")
}

#[test]
fn a_domains_range_is_a_type_argument_and_max_is_exact() {
    check(&with("top() in 255 to 255: return Range(domain of Byte).max()")).unwrap();
    check(&with("bottom() in 0 to 0: return Range(domain of Byte).min()")).unwrap();
}

#[test]
fn a_written_range_is_a_type_argument_too() {
    check(&with("low() in 5 to 5: return Range(5 to 9).min()")).unwrap();
    check(&with("high() in 9 to 9: return Range(5 to 9).max()")).unwrap();
    // Negative ends are ends like any other.
    check(&with("below() in -3 to -3: return Range(-3 to 4).min()")).unwrap();
}

#[test]
fn the_bound_is_exactly_the_bound() {
    // One short of the real top is a wrong answer, not a loose one.
    assert_eq!(
        check(&with("top() in 0 to 254: return Range(domain of Byte).max()")),
        Err(SourceCheckError::Type)
    );
    assert_eq!(
        check(&with("low() in 6 to 9: return Range(5 to 9).min()")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn an_exact_bound_proves_what_a_range_alone_could_not() {
    // Because `max` is the number 255, masking with it keeps a value inside a byte.
    check(&with(
        "mask(value in 0 to 100000) Byte: return (value & Range(domain of Byte).max()) as Byte",
    ))
    .unwrap();
}

#[test]
fn an_empty_range_is_not_a_range() {
    assert_eq!(
        check(&with("bad() in 0 to 9: return Range(9 to 5).min()")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_range_is_not_read_from_something_that_is_not_one() {
    assert_eq!(
        check(&with("Pair has\n    first Byte\n    second Byte\n\nbad() in 0 to 255: return #lower(Pair)")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn the_queries_read_a_domain_directly() {
    check(&with("top() in 255 to 255: return #upper(Byte)")).unwrap();
    check(&with("bottom() in 0 to 0: return #lower(Byte)")).unwrap();
}

#[test]
fn an_unknown_query_is_still_unsupported() {
    assert_eq!(
        check(&with("bad() in 0 to 9: return #middle(Byte)")),
        Err(SourceCheckError::Unsupported)
    );
}

#[test]
fn a_range_parameter_is_not_known_inside_its_type() {
    // Inside `Range(r)` the ends are some numbers of `r`, not a particular one.
    assert_eq!(
        check(
            "Span(r) has\n    value r\n\n    first() in 0 to 0: return #lower(r)\n"
        ),
        Err(SourceCheckError::Type)
    );
}
