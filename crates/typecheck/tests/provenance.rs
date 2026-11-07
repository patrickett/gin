//! The checked file says why a body was declined, which calls were folded into values, and
//! which trait a method came from.
use typecheck::{CheckedSourceCallee, CheckedSourceFile, DeclineKind, Limits, check_file};

fn check(text: &str) -> CheckedSourceFile {
    check_file(&parser::parse_strict(text).expect("parses"), Limits::default()).expect("checks")
}

fn index(file: &CheckedSourceFile, name: &str) -> u32 {
    file.functions.iter().position(|f| f.name == name).unwrap() as u32
}

fn decline<'a>(file: &'a CheckedSourceFile, name: &str) -> Option<&'a typecheck::BodyDecline> {
    let at = index(file, name);
    file.declines.iter().find(|(i, _)| *i == at).map(|(_, d)| d)
}

#[test]
fn the_operators_that_were_declined_are_recorded_now() {
    // Division, remainder, the bit operations and shifts used to leave a body with a decline
    // naming the operator (`DeclineKind::Operator`); the recorder records them (E8), so there is
    // none. Negative control for the decline test below: a recorded body has no entry.
    for operator in ["/", "%", "&", "|", "<<"] {
        let file = check(&format!("f(a in 0 to 3, b in 1 to 3) in 0 to 255: return a {operator} b\n"));
        assert!(file.bodies[0].is_some(), "{operator}");
        assert!(file.declines.is_empty(), "{operator}: {:?}", file.declines);
    }
}

#[test]
fn a_declined_statement_is_located() {
    let text = "answer(n in 0 to 3) in 0 to 9:\n  i in 0 to 9: 0\n  while i < n and i < 2 do\n    i :: i + 1\n  return i\n";
    let file = check(text);
    let d = decline(&file, "answer").expect("a decline");
    assert!(d.span.is_some(), "{d:?}");
    assert!(matches!(d.kind, DeclineKind::Statement | DeclineKind::Expression), "{d:?}");
}

#[test]
fn a_recorded_body_and_a_bodiless_function_have_no_decline() {
    let file = check("#unrestricted\nNothing has\n\n#instruction('brk #0')\ntrap(input Type, output Type, inputs input) output\n\nok(a in 0 to 9) in 0 to 9: return a\n");
    assert!(file.declines.is_empty(), "{:?}", file.declines);
    assert!(decline(&file, "ok").is_none());
    assert!(decline(&file, "trap").is_none());
}

#[test]
fn a_folded_call_is_remembered_and_an_unfolded_one_is_not() {
    let file = check("five() in 0 to 9: return 5\nsix() in 0 to 9: return five() + 1\nid(n in 0 to 9) in 0 to 9: return n\nthrough(n in 0 to 9) in 0 to 9: return id(n)\n");
    let five = index(&file, "five");
    assert_eq!(file.folds, vec![(index(&file, "six"), CheckedSourceCallee::Local { function: five })]);
    // The body of `six` is a constant: the dependency exists only in `folds`.
    assert!(file.folds.iter().all(|(caller, _)| *caller != index(&file, "through")));
}

#[test]
fn a_method_added_by_a_trait_says_which_and_one_the_type_wrote_does_not() {
    let file = check("Measure has\n    measure(self) in 0 to 100: return 5\n\n#unrestricted\nSmall is in 0 to 9\nSmall has Measure\n\n#unrestricted\nBig is in 0 to 9\nBig has Measure\n    measure(self) in 0 to 100: return 50\n\nfree() in 0 to 9: return 1\n");
    let by = |name: &str| file.functions[index(&file, name) as usize].provided_by.clone();
    assert_eq!(by("Small.measure").as_deref(), Some("Measure"));
    assert_eq!(by("Big.measure"), None);
    assert_eq!(by("free"), None);
}
