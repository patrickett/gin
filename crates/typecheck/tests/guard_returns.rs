//! `when comparison then return v` at the start of a body.
//!
//! Each guard splits the checker's replay into a path that returns and a path that goes on, so
//! `n` guards cost `n + 1` worlds. Facts follow the split: the comparison holds in the block and
//! its negation holds for the rest of the body.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.guards", &source, &[], Limits::default())
}

#[test]
fn a_guard_that_returns_checks() {
    let checked = check("f(n in 0 to 9) in 0 to 9:\n    when n < 2 then return 0\n    return n\n");
    assert!(checked.is_ok(), "{:?}", checked.err());
}

/// The rest of the body knows the guard failed: `n >= 2`, so `n - 2` stays inside `0 to 7`.
#[test]
fn the_negated_guard_holds_after_it() {
    let source =
        "f(n in 0 to 9) in 0 to 7:\n    when n < 2 then return 0\n    return n - 2\n";
    let checked = check(source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

/// Negative control for the test above: without the guard `n - 2` can be negative.
#[test]
fn without_the_guard_the_same_subtraction_is_refused() {
    assert!(check("f(n in 0 to 9) in 0 to 7:\n    return n - 2\n").is_err());
}

/// The comparison holds inside the block: `n > 5` makes `n - 5` at least 1 and at most 4.
#[test]
fn the_guard_holds_inside_its_block() {
    let source = "f(n in 0 to 9) in 0 to 4:\n    when n > 5 then return n - 5\n    return 0\n";
    let checked = check(source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

#[test]
fn a_returned_value_outside_the_result_domain_is_refused() {
    assert!(
        check("f(n in 0 to 9) in 0 to 9:\n    when n > 5 then return n + 10\n    return n\n")
            .is_err()
    );
}

#[test]
fn the_block_may_bind_before_it_returns() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then\n        base: n + 1\n        return base\n    return n\n";
    let checked = check(source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

#[test]
fn a_bad_statement_before_the_return_is_refused() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then\n        base in 0 to 9: n + 100\n        return 0\n    return n\n";
    assert!(check(source).is_err());
}

#[test]
fn several_guards_each_narrow_what_follows() {
    let source = "\
f(n in 0 to 9) in 0 to 3:
    when n < 3 then return 0
    when n < 6 then return 1
    when n < 9 then return 2
    return n - 6
";
    let checked = check(source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

/// Worlds grow by one per guard, not doubling: forty guards check under the default limits.
#[test]
fn many_guards_stay_linear() {
    let mut source = String::from("f(n in 0 to 99) in 0 to 99:\n");
    for bound in 0..40 {
        source.push_str(&format!("    when n = {bound} then return {bound}\n"));
    }
    source.push_str("    return n\n");
    let checked = check(&source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

#[test]
fn a_guard_with_nothing_after_it_is_not_yet_supported() {
    assert!(matches!(
        check("f(n in 0 to 9) in 0 to 9:\n    when n < 2 then return 0\n"),
        Err(SourceCheckError::Unsupported)
    ));
}

#[test]
fn a_guard_with_an_else_is_not_yet_supported() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then\n        return 0\n    else\n        return 1\n    return n\n";
    assert!(matches!(check(source), Err(SourceCheckError::Unsupported)));
}

/// A block that falls through is a join (fall_through_joins.rs), not a guard.
#[test]
fn a_block_that_falls_through_is_a_join_not_a_guard() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    m: n\n    when n < 2 then m :: 5\n    return m\n";
    assert!(check(source).is_ok());
}

fn recorded(text: &str, function: &str) -> Option<Vec<typecheck::CheckedSourceBodyKind>> {
    let module = check(text).expect("checks");
    let file = module.checked();
    let index = file.functions.iter().position(|f| f.name == function)?;
    let body = file.bodies[index].as_ref()?;
    Some(body.expressions.iter().map(|e| e.kind.clone()).collect())
}

/// The guard becomes a `Select`: the returned value when the comparison holds, the rest of the
/// body's value when it does not.
#[test]
fn a_guard_is_recorded_as_a_select() {
    let kinds = recorded(
        "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then return 0\n    return n\n",
        "f",
    )
    .expect("the body is recorded");
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, typecheck::CheckedSourceBodyKind::Select { .. })),
        "{kinds:?}"
    );
}

/// The recorder has no guard facts of its own: `n - 2` is typed from `n in 0 to 9`, below the
/// result's `0 to 7`. Recording must still succeed, because the checker proved it.
#[test]
fn a_value_proved_by_the_guard_is_still_recorded() {
    let kinds = recorded(
        "f(n in 0 to 9) in 0 to 7:\n    when n < 2 then return 0\n    return n - 2\n",
        "f",
    );
    assert!(kinds.is_some(), "the body was not recorded");
}

/// A call with known arguments folds the whole guard away.
#[test]
fn a_known_call_folds_through_the_guard() {
    let text = "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then return 7\n    return n\n\ng() in 0 to 9:\n    return f(1)\n\nh() in 0 to 9:\n    return f(5)\n";
    let module = check(text).expect("checks");
    let file = module.checked();
    for (name, expected) in [("g", 7), ("h", 5)] {
        let index = file.functions.iter().position(|f| f.name == name).unwrap();
        let body = file.bodies[index].as_ref().expect("recorded");
        assert!(
            body.expressions
                .iter()
                .all(|e| !matches!(e.kind, typecheck::CheckedSourceBodyKind::Call { .. })),
            "{name}: {:?}",
            body.expressions
        );
        assert!(
            body.expressions.iter().any(|e| matches!(
                &e.kind,
                typecheck::CheckedSourceBodyKind::Integer(value) if *value == expected.into()
            )),
            "{name}: {:?}",
            body.expressions
        );
    }
}

/// `a or b`: when it fails both comparisons failed, so `n` lies in `2 to 7`.
#[test]
fn a_failed_or_negates_every_comparison() {
    let source = "f(n in 0 to 9) in 0 to 5:\n    when n < 2 or n > 7 then return 0\n    return n - 2\n";
    let checked = check(source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

/// Negative control: only one comparison failing leaves `n - 2` open to negatives.
#[test]
fn one_comparison_alone_is_not_enough_after_an_or() {
    let source = "f(n in 0 to 9) in 0 to 5:\n    when n < 2 then return 0\n    return n - 2\n";
    assert!(check(source).is_err());
}

/// `a and b`: inside the block both held, so `n` lies in `3 to 5`.
#[test]
fn an_and_assumes_every_comparison_inside_its_block() {
    let source =
        "f(n in 0 to 9) in 0 to 2:\n    when n > 2 and n < 6 then return n - 3\n    return 0\n";
    let checked = check(source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

/// Soundness: after a failed `and` either comparison may have failed, so no fact is gained.
#[test]
fn a_failed_and_gains_no_fact() {
    let source =
        "f(n in 0 to 9) in 0 to 6:\n    when n > 2 and n < 6 then return 0\n    return n - 3\n";
    assert!(check(source).is_err());
}

#[test]
fn mixing_and_with_or_is_not_yet_supported() {
    let source =
        "f(n in 0 to 9) in 0 to 9:\n    when n < 2 or n > 7 and n < 9 then return 0\n    return n\n";
    assert!(matches!(check(source), Err(SourceCheckError::Unsupported)));
}

/// Twenty comparisons in one `or` check under the default limits.
#[test]
fn a_long_or_stays_linear() {
    let terms: Vec<String> = (0..20).map(|k| format!("n = {}", k * 2)).collect();
    let source = format!(
        "f(n in 0 to 99) in 0 to 99:\n    when {} then return 0\n    return n\n",
        terms.join(" or ")
    );
    let checked = check(&source);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

#[test]
fn and_and_or_conditions_are_recorded() {
    for condition in ["n < 2 or n > 7", "n > 2 and n < 6"] {
        let kinds = recorded(
            &format!("f(n in 0 to 9) in 0 to 9:\n    when {condition} then return 0\n    return n\n"),
            "f",
        )
        .expect("the body is recorded");
        let selects = kinds
            .iter()
            .filter(|kind| matches!(kind, typecheck::CheckedSourceBodyKind::Select { .. }))
            .count();
        // Each test is a 1-or-0 choice (two), counted into one comparison that picks the arm (one):
        // the arms belong to that single choice, so a call in one runs only when it is taken.
        assert_eq!(selects, 3, "{condition}: {kinds:?}");
    }
}
