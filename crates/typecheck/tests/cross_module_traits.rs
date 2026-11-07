//! A trait of another module: a capsule keeps its exported traits' declarations, and an
//! importer that names one gets it as its own, so `Type has Trait` works as in the defining
//! module. A trait the importer does not name is still unknown.
use typecheck::{
    CheckedSourceModule, InfixOperator, Limits, SourceCheckError, check_module,
};

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module(name, &source, dependencies, Limits::default())
}

const OPS: &str = "\
Closes has
    close(self)

Numeric has
    #infix('+')
    add(self, rhs Self):
        a in Self: self
        b in Self: rhs
        return a + b

Add(result) has
    #infix('+')
    add(self, rhs Self) result

Width has
    width(self) in 0 to 9: return 4
";

fn ops() -> CheckedSourceModule {
    check("lib.ops", OPS, &[]).unwrap()
}

fn function<'a>(module: &'a CheckedSourceModule, name: &str) -> &'a typecheck::CheckedSourceFunction {
    module
        .checked()
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function {name}"))
}

#[test]
fn a_trait_of_another_module_can_be_provided() {
    let client = "use lib.ops Closes\nCount is in 0 to 9\nCount has Closes\n    close(self): return\n";
    let checked = check("app", client, &[&ops()]).unwrap();
    assert!(checked.checked().functions.iter().any(|f| f.name == "Count.close"));
}

#[test]
fn a_required_method_must_be_written() {
    let client = "use lib.ops Closes\nCount is in 0 to 9\nCount has Closes\n";
    assert_eq!(check("app", client, &[&ops()]).err(), Some(SourceCheckError::MissingBody));
}

#[test]
fn a_default_body_is_used_and_its_result_inferred_per_provider() {
    let client = "use lib.ops Numeric\nCount is in 0 to 9\nCount has Numeric\nWide is in 0 to 99\nWide has Numeric\n";
    let checked = check("app", client, &[&ops()]).unwrap();
    assert!(function(&checked, "Count.add").infix == Some(InfixOperator::Add));
    assert!(function(&checked, "Wide.add").infix == Some(InfixOperator::Add));
}

#[test]
fn an_infix_operator_of_an_imported_trait_is_usable() {
    let client = "use lib.ops Numeric\nCount is in 0 to 9\nCount has Numeric\ntotal(a Count, b Count) in 0 to 18: return a + b\n";
    check("app", client, &[&ops()]).unwrap();
    let narrow = client.replace("0 to 18", "0 to 17");
    assert!(check("app", &narrow, &[&ops()]).is_err());
}

#[test]
fn a_parameterized_imported_trait_takes_its_arguments() {
    // The result of `add` is the trait's argument, `Wide`: left off, it is supplied.
    let client = "use lib.ops Add\nCount is in 0 to 9\n#unrestricted\nWide is in 0 to 18\nCount has Add(Wide)\n    add(self, rhs Self):\n        a in Self: self\n        b in Self: rhs\n        return (a + b) as Wide\n";
    let checked = check("app", client, &[&ops()]).unwrap();
    assert_eq!(function(&checked, "Count.add").infix, Some(InfixOperator::Add));
    let wrong = "use lib.ops Add\nCount is in 0 to 9\nCount has Add\n    add(self, rhs Self) in 0 to 18: return 0\n";
    assert_eq!(check("app", wrong, &[&ops()]).err(), Some(SourceCheckError::Type));
}

#[test]
fn a_bound_may_name_an_imported_trait() {
    let client = "use lib.ops Width\nMine is in 0 to 9\nMine has Width\nBox(element has Width) has\n    item element\nhold(x Mine) Box(Mine): return Box(item: x)\n";
    check("app", client, &[&ops()]).unwrap();
    let bad = client.replace("Mine has Width\n", "");
    assert!(check("app", &bad, &[&ops()]).is_err());
}

#[test]
fn a_trait_the_importer_did_not_name_is_unknown() {
    let client = "use lib.ops Closes\nCount is in 0 to 9\nCount has Numeric\n";
    assert_eq!(check("app", client, &[&ops()]).err(), Some(SourceCheckError::UnknownName));
    let none = "Count is in 0 to 9\nCount has Closes\n    close(self): return\n";
    assert_eq!(check("app", none, &[&ops()]).err(), Some(SourceCheckError::UnknownName));
}

#[test]
fn an_unknown_member_of_the_module_is_still_unknown() {
    let client = "use lib.ops Missing\n";
    assert_eq!(check("app", client, &[&ops()]).err(), Some(SourceCheckError::UnknownName));
}

#[test]
fn a_local_name_clashing_with_an_imported_trait_is_a_duplicate() {
    let client = "use lib.ops Closes\nCloses has\n    close(self)\n";
    assert_eq!(check("app", client, &[&ops()]).err(), Some(SourceCheckError::Duplicate));
}

#[test]
fn the_importer_does_not_reexport_the_trait() {
    let middle = check("mid", "use lib.ops Closes\nCount is in 0 to 9\nCount has Closes\n    close(self): return\n", &[&ops()]).unwrap();
    let top = "use mid Closes\n";
    assert_eq!(check("app", top, &[&middle]).err(), Some(SourceCheckError::UnknownName));
}
