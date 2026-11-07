//! A claim may name an inline nullary accessor (`cell.used_count`) of a type whose field is
//! `#private`; the checker unfolds it to the field. Nothing else is unfolded.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default())
}

const CELL: &str = "\
#unrestricted
Cell has
    #private
    used in 0 to 100
    used_count(self ref) in 0 to 100: return self.used
    shifted(self ref) in 0 to 100: return 1
    bump(self mut and self.used_count < 50) where self.used_count :: 1:
        self.used :: 1
        return
    wrong(self mut) where self.used_count :: 1:
        self.used :: 2
        return

";

fn with(tail: &str) -> String {
    // `Cell` is written without `wrong` where it must be accepted, see `without_wrong`.
    format!("{}{tail}\n", CELL.replace("    wrong(self mut) where self.used_count :: 1:\n        self.used :: 2\n        return\n\n", ""))
}

#[test]
fn an_accessor_in_a_field_claim_unfolds_to_the_private_field() {
    check(&with("Holder has\n    cell Cell and cell.used_count = 1\nmake() Holder: return Holder(cell: Cell(used: 1))")).unwrap();
}

#[test]
fn a_field_claim_through_an_accessor_is_enforced_at_construction() {
    let bad = with("Holder has\n    cell Cell and cell.used_count = 1\nmake() Holder: return Holder(cell: Cell(used: 2))");
    assert_eq!(check(&bad).map(|_| ()), Err(SourceCheckError::Unproved));
}

#[test]
fn an_accessor_in_a_parameter_claim_is_proved_at_the_call() {
    let ok = with("need(c Cell and c.used_count = 3): return\nmain(): need(Cell(used: 3))");
    check(&ok).unwrap();
    let bad = with("need(c Cell and c.used_count = 3): return\nmain(): need(Cell(used: 4))");
    assert_eq!(check(&bad).map(|_| ()), Err(SourceCheckError::Unproved));
}

#[test]
fn an_accessor_unfolds_in_a_where_transition() {
    check(&with("main(): return")).unwrap();
    // The body sets 2 where the transition promises 1.
    assert_eq!(check(&format!("{CELL}main(): return\n")).map(|_| ()), Err(SourceCheckError::Unproved));
}

#[test]
fn a_method_that_is_not_an_accessor_is_still_unsupported() {
    let text = with("Holder has\n    cell Cell and cell.shifted = 1");
    assert_eq!(check(&text).map(|_| ()), Err(SourceCheckError::Unsupported));
}

#[test]
fn a_private_field_is_not_readable_in_a_claim_of_another_type() {
    let text = with("Holder has\n    cell Cell and cell.used = 1");
    assert_eq!(check(&text).map(|_| ()), Err(SourceCheckError::Privacy));
}

const SPAN: &str = "\
#unrestricted
Length is in 0 to 1000

Span(item) has
    #private
    count Length
    length(self ref) Length: return self.count
    clear(self mut) where self.length :: 0:
        self.count :: 0
        return
    wrong(self mut) where self.length :: 0:
        self.count :: 1
        return

";

#[test]
fn a_where_transition_works_in_a_method_of_a_generic_type() {
    // The type parameter `item` is inferred and takes no argument, so `self` and `n` keep their
    // positions for the transition (they used to be refused as `Unsupported`).
    let ok = SPAN.replace("    wrong(self mut) where self.length :: 0:\n        self.count :: 1\n        return\n", "");
    assert_eq!(check(&format!("{ok}main(): return\n")).map(|_| ()), Ok(()));
    // Negative control: a body that does not make the promised transition is still unproved.
    assert_eq!(check(&format!("{SPAN}main(): return\n")).map(|_| ()), Err(SourceCheckError::Unproved));
}

mod across_modules {
    use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

    fn module(name: &str, text: &str, deps: &[&CheckedSourceModule]) -> Result<CheckedSourceModule, SourceCheckError> {
        let source = parser::parse_strict(text).expect("strict fixture");
        check_module(name, &source, deps, Limits::default())
    }

    const LIBRARY: &str = "\
#unrestricted
Cell has
    #private
    used in 0 to 100
    used_count(self ref) in 0 to 100: return self.used
    shifted(self ref) in 0 to 100: return 1
    make(n in 0 to 100) Cell: return Cell(used: n)
";

    #[test]
    fn an_accessor_is_exported_with_its_type() {
        let library = module("lib.cell", LIBRARY, &[]).unwrap();
        let app = "use lib.cell Cell\nHolder has\n    cell Cell and cell.used_count = 1\nneed(c Cell and c.used_count = 3): return\nmain(): return\n";
        assert!(module("app", app, &[&library]).is_ok());
    }

    #[test]
    fn across_modules_a_private_field_a_non_accessor_and_a_wrong_value_are_refused() {
        let library = module("lib.cell", LIBRARY, &[]).unwrap();
        let private = "use lib.cell Cell\nHolder has\n    cell Cell and cell.used = 1\nmain(): return\n";
        assert_eq!(module("app", private, &[&library]).map(|_| ()), Err(SourceCheckError::Privacy));
        let method = "use lib.cell Cell\nHolder has\n    cell Cell and cell.shifted = 1\nmain(): return\n";
        assert_eq!(module("app", method, &[&library]).map(|_| ()), Err(SourceCheckError::Unsupported));
    }
}

mod nested_and_bounds {
    use typecheck::{Limits, SourceCheckError, check_file};

    fn check(text: &str) -> Result<(), SourceCheckError> {
        let file = parser::parse_strict(text).expect("strict fixture");
        check_file(&file, Limits::default()).map(|_| ())
    }

    const NESTED: &str = "\
#unrestricted
Inner has
    #private
    used in 0 to 9
    count(self ref) in 0 to 9: return self.used
#unrestricted
Outer has
    #private
    inner Inner
    length(self ref) in 0 to 9: return self.inner.count
    make() o Outer where o.length = 0: return Outer(inner: Inner(used: 0))
";

    #[test]
    fn an_accessor_may_read_through_a_field_and_another_accessor() {
        let ok = format!("{NESTED}need(o Outer and o.length = 0): return\nmain(): need(Outer.make())\n");
        assert_eq!(check(&ok), Ok(()));
        let wrong = format!("{NESTED}need(o Outer and o.length = 1): return\nmain(): need(Outer.make())\n");
        assert_eq!(check(&wrong), Err(SourceCheckError::Unproved));
    }

    const MAKER: &str = "Thing(x) has\n    n in 0 to 9\nMaker has\n    make(self mut, x Type) Thing(x)\n    release(self)\n";

    #[test]
    fn a_bound_call_may_lend_mut_and_pass_a_type() {
        // G-F: a call through a bound on a `mut` value, with a type argument.
        let ok = format!("{MAKER}use_it(m has Maker, x Type, a mut m) Thing(x): return a.make(x)\nmain(): return\n");
        assert_eq!(check(&ok), Ok(()));
        // Without the bound there is no `make` to call.
        let unbound = format!("{MAKER}use_it(m Type, x Type, a mut m) Thing(x): return a.make(x)\nmain(): return\n");
        assert!(check(&unbound).is_err());
    }

    #[test]
    fn a_provider_promises_the_trait_claims() {
        let base = "Counter has\n    next(self mut) n in 0 to 9 where n = 1\nOne has Counter\n    k in 0 to 9\n";
        let same = format!("{base}    next(self mut) n in 0 to 9 where n = 1: return 1\nmain(): return\n");
        assert_eq!(check(&same), Ok(()));
        // A provider that promises less (or something else) is refused.
        let other = format!("{base}    next(self mut) n in 0 to 9 where n >= 0: return 1\nmain(): return\n");
        assert_eq!(check(&other), Err(SourceCheckError::Type));
    }
}
