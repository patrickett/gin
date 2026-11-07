//! `#wrap` on a named integer domain: converting an out-of-range integer into the nominal
//! domain takes the congruent value inside it. Arithmetic on domain values comes from a
//! `Numeric` trait, since nominal values have no implicit arithmetic.
use i256::I256;
use typecheck::{
    Capability, CheckedSourceFile, Limits, SourceCheckError, check_file, wrap_into,
};

const NUMERIC: &str = "\
Numeric has
    #infix('+')
    add(self, rhs Self):
        a in Self: self
        b in Self: rhs
        return a + b
";

fn domain(markers: &str, name: &str, upper: &str) -> String {
    format!("{markers}{name} is in 0 to {upper}\n{name} has Numeric\n")
}

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict wrap fixture");
    check_file(&file, Limits::default())
}

fn program(domains: &str, tail: &str) -> String {
    format!("{NUMERIC}{domains}{tail}")
}

const ADD: &str = "add(left Register, right Register) Register:
    Register(a) := left
    Register(b) := right
    return Register(contents: a + b)
";

#[test]
fn a_named_domain_is_its_own_type_even_with_equal_bounds() {
    let two = format!(
        "{}{}",
        domain("#unrestricted\n", "Byte", "255"),
        domain("#unrestricted\n", "Other", "255")
    );
    assert!(check(&program(&two, "identity(x Byte) Byte: return x\n")).is_ok());
    assert_eq!(
        check(&program(&two, "mix(x Byte) Other: return x\n")),
        Err(SourceCheckError::Type)
    );
    // An `#unrestricted` domain is a number with a name: it is read as its number anywhere (a
    // linear domain keeps its number to its own methods, tested in
    // `open_domains_and_element_writes.rs`).
    let widen = program(&two, "widen(x Byte) in 0 to 1000: return x\n");
    assert!(check(&widen).is_ok());
}

#[test]
fn an_unrestricted_domain_is_copyable_and_needs_no_terminal() {
    let byte = domain("#unrestricted\n", "Byte", "255");
    let checked = check(&program(
        &byte,
        "twice(x Byte) Byte:\n  a: x\n  b: x\n  return b\n",
    ))
    .unwrap();
    assert_eq!(checked.aggregates[0].capability, Capability::Unrestricted);
    assert!(!checked.aggregates[0].wrapping);
}

#[test]
fn without_wrap_the_overflow_is_refused() {
    let word = format!(
        "{}Register has contents Word\n",
        domain("#unrestricted\n", "Word", "18446744073709551615")
    );
    assert_eq!(check(&program(&word, ADD)), Err(SourceCheckError::Unproved));
}

#[test]
fn arithmetic_stored_in_a_wrap_domain_is_accepted() {
    let word = format!(
        "{}Register has contents Word\n",
        domain("#unrestricted, #wrap\n", "Word", "18446744073709551615")
    );
    let checked = check(&program(&word, ADD)).unwrap();
    let kinds: Vec<_> = checked.aggregates.iter().map(|a| a.wrapping).collect();
    assert_eq!(kinds, [true, false], "Word wraps; Register does not");
}

#[test]
fn a_return_wraps_into_the_result_domain_only_when_the_domain_wraps() {
    let inc = "inc(x Byte) Byte: return x + 1\n";
    let wrapping = domain("#unrestricted, #wrap\n", "Byte", "255");
    assert!(check(&program(&wrapping, inc)).is_ok());
    let plain = domain("#unrestricted\n", "Byte", "255");
    assert_eq!(
        check(&program(&plain, inc)),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_known_value_outside_the_range_wraps_to_its_residue() {
    let wrapping = domain("#unrestricted, #wrap\n", "Byte", "255");
    for tail in [
        "f() Byte: return 256\n",
        "f() Byte: return 0 - 1\n",
        "f() Byte: return 1000\n",
    ] {
        assert!(check(&program(&wrapping, tail)).is_ok(), "{tail}");
    }
    let plain = domain("#unrestricted\n", "Byte", "255");
    assert_eq!(
        check(&program(&plain, "f() Byte: return 256\n")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn wrapping_is_orthogonal_to_linearity() {
    let linear = "#wrap\nByte is in 0 to 255\nByte has\n    close(self): return\n";
    let checked = check(linear).unwrap();
    assert!(checked.aggregates[0].wrapping);
    assert_eq!(checked.aggregates[0].capability, Capability::Linear);
    assert_eq!(
        check("#wrap\nByte is in 0 to 255\n"),
        Err(SourceCheckError::MissingTerminal)
    );
}

#[test]
fn the_attribute_marks_the_integer_domain_that_follows() {
    for (text, error) in [
        (
            "#wrap\n#wrap\nByte is in 0 to 255\n",
            SourceCheckError::Duplicate,
        ),
        // A refinement on a domain is accepted now (it is checked at each conversion), so the
        // declaration is judged on its other merits: this one has no way to be ended.
        (
            "#wrap\nByte is in 0 to 255 and > 3\n",
            SourceCheckError::MissingTerminal,
        ),
        ("Window is 0 to 16\n", SourceCheckError::Unsupported),
    ] {
        assert_eq!(check(text).err(), Some(error), "{text}");
    }
}

/// `#wrap` has a meaning only on a named integer domain, so anywhere else it is a flaw.
#[test]
fn wrap_on_anything_but_an_integer_domain_is_a_wrap_target_flaw() {
    for text in [
        "#wrap\nPair has first in 0 to 1\n",
        "#wrap\nEmpty has\nFull has v in 0 to 1\n",
        "Empty has\nFull has v in 0 to 1\n#wrap\nSlot is Empty or Full\n",
        "#wrap\nf() in 0 to 1: return 0\n",
        "#wrap\nuse other.module Thing\n",
        "#wrap\n",
        "Byte is in 0 to 255\n#wrap\n",
    ] {
        assert_eq!(
            check(text).err(),
            Some(SourceCheckError::WrapTarget),
            "{text}"
        );
    }
}

#[test]
fn wrap_into_is_congruence_modulo_the_domain_width() {
    let n = |value: i64| I256::from(value);
    assert_eq!(wrap_into(n(10), n(0), n(7)), Some(n(2)));
    assert_eq!(wrap_into(n(-1), n(0), n(7)), Some(n(7)));
    assert_eq!(wrap_into(n(5), n(-4), n(3)), Some(n(-3)));
    assert_eq!(wrap_into(n(-9), n(-4), n(3)), Some(n(-1)));
}
