//! A failure names the exact expression being checked and the facts proved in scope; a hole
//! names the facts that hold where it stands (the guards taken to reach it included).
use i256::I256;
use typecheck::{
    Limits, SourceCheckError, SourceCheckFailure, SourceCheckNoteKind, SourceFact, check_module_diagnostic,
};

fn fail(text: &str) -> SourceCheckFailure {
    let file = parser::parse_strict(text).expect("parses");
    check_module_diagnostic("m", &file, &[], Limits::default()).expect_err("fails")
}

fn hole_facts(text: &str) -> Vec<SourceFact> {
    let file = parser::parse_strict(text).expect("parses");
    let module = check_module_diagnostic("m", &file, &[], Limits::default()).expect("checks");
    let notes: Vec<_> = module.checked().notes.iter().filter(|n| n.kind == SourceCheckNoteKind::Hole).collect();
    assert_eq!(notes.len(), 1);
    notes[0].facts.clone()
}

fn interval(name: &str, lower: i64, upper: i64) -> SourceFact {
    SourceFact::Interval { name: name.to_owned(), lower: I256::from(lower), upper: I256::from(upper) }
}

fn culprit_text<'a>(text: &'a str, failure: &SourceCheckFailure) -> &'a str {
    let c = failure.culprit.expect("a culprit");
    &text[c.start()..c.end()]
}

#[test]
fn the_failing_expression_is_located_exactly_inside_the_construct() {
    let text = "f(n in 0 to 9) in 0 to 9: return n - 3\n";
    let failure = fail(text);
    assert_eq!(failure.error, SourceCheckError::Type);
    let span = failure.span.unwrap();
    assert_eq!(&text[span.start()..span.end()], "f(n in 0 to 9) in 0 to 9: return n - 3");
    assert_eq!(culprit_text(text, &failure), "n - 3");
}

#[test]
fn a_call_argument_that_may_leave_its_domain_is_the_culprit() {
    let text = "g(a in 0 to 3) in 0 to 3: return a\nf(n in 0 to 9) in 0 to 3:\n    k in 0 to 9: n\n    return g(k)\n";
    let failure = fail(text);
    assert_eq!(culprit_text(text, &failure), "g(k)");
}

#[test]
fn the_facts_are_those_of_the_failing_function_only() {
    let text = "g(a in 0 to 3) in 0 to 3: return a\nf(n in 0 to 9) in 0 to 3:\n    k in 0 to 9: n\n    return g(k)\n";
    let facts = fail(text).facts;
    assert!(facts.contains(&interval("n", 0, 9)), "{facts:?}");
    assert!(facts.contains(&interval("k", 0, 9)), "{facts:?}");
    // `k` is `n`, said as an offset of zero.
    assert!(facts.contains(&SourceFact::Offset { name: "k".into(), base: "n".into(), offset: I256::ZERO }), "{facts:?}");
    // Negative control: nothing of `g` leaks into the facts of `f`.
    assert!(!facts.iter().any(|f| matches!(f, SourceFact::Interval { name, .. } | SourceFact::Known { name, .. } if name == "a")), "{facts:?}");
}

#[test]
fn a_value_known_exactly_is_a_known_fact_not_an_interval() {
    let text = "f(n in 0 to 9) in 0 to 3:\n    k in 0 to 9: 7\n    return k + n\n";
    let facts = fail(text).facts;
    assert!(facts.contains(&SourceFact::Known { name: "k".into(), value: I256::from(7) }), "{facts:?}");
    assert!(!facts.iter().any(|f| matches!(f, SourceFact::Interval { name, .. } if name == "k")), "{facts:?}");
}

#[test]
fn a_hole_carries_the_guards_taken_to_reach_it() {
    let facts = hole_facts("f(n in 0 to 9) in 0 to 9:\n    when n < 5 then return 1\n    return #todo\n");
    assert_eq!(facts, vec![interval("n", 5, 9)]);
}

#[test]
fn a_hole_carries_relations_between_names() {
    let facts = hole_facts("f(n in 0 to 9, m in 0 to 9) in 0 to 9:\n    when n < m then\n        return #todo\n    return 1\n");
    assert!(facts.contains(&interval("n", 0, 8)), "{facts:?}");
    assert!(facts.contains(&interval("m", 1, 9)), "{facts:?}");
    assert!(facts.contains(&SourceFact::Relation { left: "n".into(), operator: "<", right: "m".into() }), "{facts:?}");
}

#[test]
fn a_hole_before_any_guard_has_only_the_declared_domain() {
    // Negative control: nothing is narrowed that was not.
    let facts = hole_facts("f(n in 0 to 9) in 0 to 9: return #todo\n");
    assert_eq!(facts, vec![interval("n", 0, 9)]);
}

#[test]
fn other_notes_and_unlocated_failures_publish_no_facts() {
    let file = parser::parse_strict("Small is in 0 to 255\nSmall has finish(self): return\n").unwrap();
    let module = check_module_diagnostic("m", &file, &[], Limits::default()).unwrap();
    assert!(module.checked().notes.iter().all(|n| n.facts.is_empty()));
    // A failure before any expression is visited has no culprit.
    let failure = fail("Thing has\n    a in 0 to 3\n    a in 0 to 3\n");
    assert!(failure.culprit.is_none(), "{failure:?}");
}

#[test]
fn the_culprit_always_lies_inside_the_reported_construct() {
    for text in [
        "f(n in 0 to 9) in 0 to 9: return n - 3\n",
        "f(n in 0 to 9) in 0 to 9: return missing(n)\n",
        "f(n in 0 to 9) in 0 to 3: return n\n",
    ] {
        let failure = fail(text);
        if let (Some(span), Some(culprit)) = (failure.span, failure.culprit) {
            assert!(span.start() <= culprit.start() && culprit.end() <= span.end(), "{text}: {failure:?}");
        }
    }
}
