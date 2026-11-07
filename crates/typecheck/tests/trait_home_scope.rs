//! An imported trait's declaration is read in its home module: the names it mentions mean what
//! they meant where it was written, whatever the importer has in scope.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn module(name: &str, text: &str, deps: &[&CheckedSourceModule]) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict fixture");
    check_module(name, &source, deps, Limits::default())
}

const HOME: &str = "\
#unrestricted
Step is in 0 to 9

Stepper has
    next(self mut) Step
";

fn home() -> CheckedSourceModule {
    module("lib.steps", HOME, &[]).unwrap()
}

#[test]
fn a_name_only_the_trait_uses_need_not_be_imported() {
    let library = home();
    let client = "use lib.steps Stepper\nrun(s has Stepper, a mut s):\n    a.next\n    return\nmain(): return\n";
    assert!(module("app", client, &[&library]).is_ok(), "{:?}", module("app", client, &[&library]).err());
}

#[test]
fn the_importers_own_name_does_not_leak_into_the_trait() {
    let library = home();
    // The importer's `Step` is another type; the trait still means the home one.
    let mixed = "use lib.steps Stepper\n#unrestricted\nStep is in 0 to 3\nrun(s has Stepper, a mut s) in 0 to 3:\n    got Step: a.next\n    return got\nmain(): return\n";
    assert!(module("app", mixed, &[&library]).is_err());
    // Control: without the importer's declaration the result is the home `Step`, and is accepted.
    let plain = "use lib.steps Stepper\nrun(s has Stepper, a mut s) in 0 to 9:\n    return a.next\nmain(): return\n";
    assert!(module("app", plain, &[&library]).is_ok(), "{:?}", module("app", plain, &[&library]).err());
}

#[test]
fn a_name_in_both_scopes_is_the_home_modules() {
    let library = home();
    // The importer also has a `Step`, with a different range: no clash, and a provider written
    // with the importer's `Step` does not satisfy the trait.
    let provider = "use lib.steps Stepper\n#unrestricted\nStep is in 0 to 3\n#unrestricted\nCounter has Stepper\n    n in 0 to 3\n    next(self mut) Step: return 1\nmain(): return\n";
    assert_eq!(module("app", provider, &[&library]).map(|_| ()), Err(SourceCheckError::Type));
}

#[test]
fn the_implementer_is_still_checked_against_the_home_types() {
    let library = home();
    // A provider returns the home `Step` (imported by name to write it).
    let ok = "use lib.steps Stepper, Step\n#unrestricted\nCounter has Stepper\n    n in 0 to 3\n    next(self mut) Step: return 1\nmain(): return\n";
    assert!(module("app", ok, &[&library]).is_ok(), "{:?}", module("app", ok, &[&library]).err());
    let wrong = "use lib.steps Stepper\n#unrestricted\nCounter has Stepper\n    n in 0 to 3\n    next(self mut) in 0 to 9: return 1\nmain(): return\n";
    assert_eq!(module("app", wrong, &[&library]).map(|_| ()), Err(SourceCheckError::Type));
}

#[test]
fn a_name_the_home_module_lacks_is_reported_at_the_trait_in_the_home_module() {
    let broken = "Stepper has\n    next(self mut) Missing\n";
    let failure = typecheck::check_module_diagnostic("lib.broken", &parser::parse_strict(broken).unwrap(), &[], Limits::default())
        .expect_err("the home module is refused");
    assert!(failure.span.is_some(), "{failure:?}");
}
