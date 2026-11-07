//! A trait is a `has` block of methods with no fields; `Type has Trait` adds its methods.
use typecheck::{
    AggregateKind, CheckedSourceFile, InfixOperator, Limits, SourceCheckError, SourceType,
    check_file,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict trait fixture");
    check_file(&file, Limits::default())
}

const NUMERIC: &str = "\
Numeric has
    #infix('+')
    add(self, rhs Self):
        a in Self: self
        b in Self: rhs
        return a + b
    #infix('*')
    multiply(self, rhs Self):
        a in Self: self
        b in Self: rhs
        return a * b
";

const COUNT: &str = "\
Count is in 0 to 9
Count has Numeric
    close(self): return
";

fn program(tail: &str) -> String {
    format!("{NUMERIC}{COUNT}{tail}")
}

fn range(lower: i64, upper: i64) -> SourceType {
    SourceType::Integer {
        lower: lower.into(),
        upper: upper.into(),
    }
}

#[test]
fn a_trait_is_not_a_type_and_its_defaults_become_methods_of_the_provider() {
    let checked = check(&program("")).unwrap();
    let kinds: Vec<_> = checked
        .aggregates
        .iter()
        .map(|a| (a.name.as_str(), a.kind))
        .collect();
    assert_eq!(kinds, [("Count", AggregateKind::Domain)]);
    let named = |name: &str| checked.functions.iter().find(|f| f.name == name).unwrap();
    assert_eq!(named("Count.add").infix, Some(InfixOperator::Add));
    assert_eq!(named("Count.multiply").infix, Some(InfixOperator::Multiply));
    assert!(checked.functions.iter().any(|f| f.name == "Count.close"));
}

#[test]
fn a_default_result_is_inferred_from_the_provider() {
    let wide =
        format!("{NUMERIC}{COUNT}Wide is in 0 to 99\nWide has Numeric\n    close(self): return\n");
    let checked = check(&wide).unwrap();
    let result = |name: &str| {
        checked
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .result
    };
    assert_eq!(result("Count.add"), range(0, 18));
    assert_eq!(result("Count.multiply"), range(0, 81));
    assert_eq!(result("Wide.add"), range(0, 198));
}

#[test]
fn operators_use_the_provided_methods() {
    assert!(
        check(&program(
            "total(a Count, b Count) in 0 to 18: return a + b\n"
        ))
        .is_ok()
    );
    assert!(
        check(&program(
            "scaled(a Count, b Count) in 0 to 81: return a * b\n"
        ))
        .is_ok()
    );
    let too_narrow = program("total(a Count, b Count) in 0 to 17: return a + b\n");
    assert!(check(&too_narrow).is_err());
    let twice = program("total(a Count) in 0 to 18: return a + a\n");
    assert_eq!(check(&twice), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn a_type_may_write_a_method_instead_of_taking_the_default() {
    let source = format!(
        "{NUMERIC}Count is in 0 to 9\nCount has Numeric\n    close(self): return\n    add(self, rhs Self) in 0 to 18:\n        a in Self: self\n        b in Self: rhs\n        return a + b\ntotal(a Count, b Count) in 0 to 18: return a + b\n"
    );
    let checked = check(&source).unwrap();
    let adds = checked
        .functions
        .iter()
        .filter(|f| f.name == "Count.add")
        .count();
    assert_eq!(adds, 1, "written once, not also provided");
    // The override keeps the operator the trait attaches to `add`.
    let add = checked
        .functions
        .iter()
        .find(|f| f.name == "Count.add")
        .unwrap();
    assert_eq!(add.infix, Some(InfixOperator::Add));
}

#[test]
fn a_required_method_without_a_default_must_be_written() {
    let trait_ = "Closes has\n    close(self)\n";
    let missing = format!("{trait_}Count is in 0 to 9\nCount has Closes\n");
    assert_eq!(check(&missing), Err(SourceCheckError::MissingBody));
    let written =
        format!("{trait_}Count is in 0 to 9\nCount has Closes\n    close(self): return\n");
    assert!(check(&written).is_ok(), "{:?}", check(&written));
}

#[test]
fn a_trait_may_supply_the_terminal_method() {
    let source = "Closes has\n    close(self): return\nCount is in 0 to 9\nCount has Closes\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn unknown_traits_and_traits_used_as_types_are_rejected() {
    let unknown = "Count is in 0 to 9\nCount has Missing\n    close(self): return\n";
    assert_eq!(check(unknown), Err(SourceCheckError::UnknownName));
    let as_type = format!("{NUMERIC}f(x Numeric): return\n");
    assert!(check(&as_type).is_err());
}

#[test]
fn two_traits_may_not_provide_the_same_method() {
    let source = "\
A has
    one(self): return
B has
    one(self): return
Count is in 0 to 9
Count has A and B
";
    assert_eq!(check(source), Err(SourceCheckError::Duplicate));
}

#[test]
fn a_trait_cannot_be_marked_unrestricted() {
    assert_eq!(
        check("#unrestricted\nCloses has\n    close(self): return\n"),
        Err(SourceCheckError::UnrestrictedTarget)
    );
}
