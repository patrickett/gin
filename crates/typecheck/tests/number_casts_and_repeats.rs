//! `number as Domain` (accepted where the range is proved to fit) and `[value; count]`
//! (copies, so the element must be `#unrestricted`).
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default())
}

const BASE: &str = "\
#unrestricted
Byte is in 0 to 255
Linear is in 0 to 255
Linear has
  release(self): return
#unrestricted
Wide is in 0 to 65535
Table has
  cells [Byte; 8]
";

fn ok(tail: &str) {
    let source = format!("{BASE}{tail}");
    assert!(check(&source).is_ok(), "{tail}: {:?}", check(&source).map(|_| ()));
}

fn bad(tail: &str, error: SourceCheckError) {
    assert_eq!(check(&format!("{BASE}{tail}")).map(|_| ()), Err(error), "{tail}");
}

#[test]
fn a_number_is_cast_to_a_domain_that_holds_it() {
    ok("f() Byte: return 200 as Byte\n");
    ok("f(w Wide and w < 256) Byte: return w as Byte\n");
    ok("f(b Byte) Wide: return b as Wide\n");
}

#[test]
fn a_cast_that_may_not_fit_is_refused() {
    bad("f() Byte: return 256 as Byte\n", SourceCheckError::Type);
    bad("f(w Wide) Byte: return w as Byte\n", SourceCheckError::Unproved);
}

#[test]
fn an_array_is_repeated_from_an_unrestricted_element() {
    ok("make() Table: return Table(cells: [0 as Byte; 8])\n");
    ok("make() Table: return Table(cells: [0 as Byte; 4 + 4])\n");
}

#[test]
fn a_repeat_has_the_length_it_says() {
    bad("make() Table: return Table(cells: [0 as Byte; 7])\n", SourceCheckError::Type);
}

#[test]
fn a_linear_element_cannot_be_repeated() {
    bad(
        "Bag has cells [Linear; 2]\nmake() Bag: return Bag(cells: [0 as Linear; 2])\n",
        SourceCheckError::Type,
    );
}
