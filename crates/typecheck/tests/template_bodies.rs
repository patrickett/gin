//! A generic function, or a method of a generic type, is recorded once as a template: its types
//! may be `Generic`, a call carries the type arguments it applies, and a call through a bound is a
//! `BoundCall` that is resolved only for a concrete type.
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceFile, Limits, SourceType, check_file,
};

fn check(text: &str) -> CheckedSourceFile {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).expect("checks")
}

fn kinds(file: &CheckedSourceFile, name: &str) -> Vec<CheckedSourceBodyKind> {
    let index = file.functions.iter().position(|f| f.name == name).expect(name);
    let body = file.bodies[index].as_ref().unwrap_or_else(|| panic!("{name} has no body"));
    body.expressions.iter().map(|e| e.kind.clone()).collect()
}

const SOURCE: &str = "Release has
    release(self)

Pair(element) has
    left element
    right element

    swap(self) Pair(element):
        Pair(left, right) := self
        Pair(left: right, right: left)

Step(element) is Yield(element) or Done

rewrap(element, step Step(element)) Step(element):
    return when step is
        Yield(value) then Yield(value)
        Done then Done

id(element, x element) element: return x

dispose(element has Release, value element):
    element.release(value)
    return

flip(element, p Pair(element)) Pair(element): p.swap()
";

#[test]
fn an_identity_function_records_with_generic_types() {
    let file = check(SOURCE);
    let index = file.functions.iter().position(|f| f.name == "id").unwrap();
    let body = file.bodies[index].as_ref().unwrap();
    assert_eq!(body.expressions[0].ty, SourceType::Generic { index: 0 });
}

#[test]
fn a_method_of_a_generic_type_records_its_destructuring_and_construction() {
    let file = check(SOURCE);
    let nodes = kinds(&file, "Pair.swap");
    assert!(nodes.iter().any(|k| matches!(k, CheckedSourceBodyKind::Field { index: 1, .. })));
    assert!(nodes.iter().any(|k| matches!(k, CheckedSourceBodyKind::Construct { .. })));
}

#[test]
fn a_call_carries_the_type_arguments_it_applies() {
    let file = check(SOURCE);
    let nodes = kinds(&file, "flip");
    assert!(nodes.iter().any(|k| matches!(
        k,
        CheckedSourceBodyKind::Call { type_arguments, .. }
            if type_arguments == &[SourceType::Generic { index: 0 }]
    )));
}

#[test]
fn a_call_through_a_bound_is_a_bound_call_and_the_body_may_have_effects() {
    let file = check(SOURCE);
    let nodes = kinds(&file, "dispose");
    assert!(nodes.iter().any(|k| matches!(
        k,
        CheckedSourceBodyKind::BoundCall { generic: 0, method, arguments, .. }
            if method == "release" && arguments == &[0]
    )));
    let index = file.functions.iter().position(|f| f.name == "dispose").unwrap();
    assert!(file.bodies[index].as_ref().unwrap().effects);
}

#[test]
fn a_match_over_a_generic_sum_records_its_arms_and_payload() {
    let file = check(SOURCE);
    let nodes = kinds(&file, "rewrap");
    assert!(nodes.iter().any(|k| matches!(k, CheckedSourceBodyKind::Match { .. })));
    assert!(nodes.iter().any(|k| matches!(k, CheckedSourceBodyKind::VariantField { .. })));
}

#[test]
fn a_non_generic_caller_records_the_concrete_type_arguments() {
    let file = check(&format!(
        "{SOURCE}#unrestricted\nWord is in 0 to 9\nuse_it(w Word) Word: id(Word, w)\n"
    ));
    let nodes = kinds(&file, "use_it");
    assert!(nodes.iter().any(|k| matches!(
        k,
        CheckedSourceBodyKind::Call { type_arguments, .. }
            if matches!(type_arguments[..], [SourceType::Aggregate { .. }])
    )));
}
