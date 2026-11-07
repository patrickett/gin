//! `#lower(T)` and `#upper(T)`: the bounds a type declares, as numbers known at compile time.
//! The compiler has no `.min` or `.max` of its own; those are library code (`range_values.rs`).
//! The queries `#size(x)` and `#index_bits(x)` parse but are not defined; `#target` is, and has
//! its own tests (`target_facts.rs`).
use typecheck::{CheckedSourceBodyKind, CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.bounds", &source, &[], Limits::default())
}

const BYTE: &str = "#unrestricted\nByte is in 0 to 255\n";

#[test]
fn the_declared_bounds_are_numbers() {
    let source = format!(
        "{BYTE}\ntop() in 255 to 255: return #upper(Byte)\nbottom() in 0 to 0: return #lower(Byte)\n"
    );
    let checked = check(&source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

/// They are exact: a result that does not hold the bound is refused.
#[test]
fn a_bound_is_exact() {
    let source = format!("{BYTE}\ntop() in 0 to 254: return #upper(Byte)\n");
    assert!(check(&source).is_err());
}

#[test]
fn a_bound_takes_part_in_arithmetic_and_folds() {
    let source = format!(
        "{BYTE}\nbelow() in 254 to 254: return #upper(Byte) - 1\n"
    );
    let module = check(&source).expect("checks");
    let file = module.checked();
    let body = file.bodies[0].as_ref().expect("recorded");
    assert!(
        body.expressions.iter().any(|e| matches!(
            &e.kind,
            CheckedSourceBodyKind::Integer(value) if *value == 254.into()
        )),
        "{:?}",
        body.expressions
    );
}

#[test]
fn a_plain_range_has_bounds_too() {
    let source = "#unrestricted\nSmall is in -3 to 40\n\nhigh() in 40 to 40: return #upper(Small)\nlow() in -3 to -3: return #lower(Small)\n";
    let checked = check(source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

/// The compiler knows no `.min` or `.max` of a domain, nor any `Range` type: they are library code.
#[test]
fn the_compiler_has_no_min_or_max() {
    for member in ["min", "max", "mid"] {
        let source = format!("{BYTE}\nf() in 0 to 255: return (domain of Byte).{member}\n");
        assert!(check(&source).is_err(), "{member}");
    }
}

#[test]
fn a_type_without_a_range_has_no_bounds() {
    let source = "Pair has\n    a in 0 to 9\n\nbad() in 0 to 9: return #upper(Pair)\n";
    assert!(check(source).is_err());
}

#[test]
fn compile_time_queries_are_not_defined_yet() {
    for body in ["#size(x)", "#size(x) + 1", "#index_bits(x)"] {
        let source = format!("f(x in 0 to 9) in 0 to 99: return {body}\n");
        assert!(
            matches!(check(&source), Err(SourceCheckError::Unsupported)),
            "{body}"
        );
    }
}
