//! `when c then <block>` whose branches fall through.
//!
//! The statement is joined once after its branches instead of being replayed once per branch,
//! so `k` of them in a row cost `k` joins, not `2^k` worlds. The join keeps what both branches
//! agree on, spans what they assigned, and drops what the comparison taught.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.joins", &source, &[], Limits::default())
}

fn ok(text: &str) {
    let checked = check(text);
    assert!(checked.is_ok(), "{text}\n{:?}", checked.err());
}

fn refused(text: &str) {
    assert!(check(text).is_err(), "{text}");
}

#[test]
fn a_block_that_assigns_and_falls_through_checks() {
    ok("f(n in 0 to 9) in 0 to 9:\n    m in 0 to 9: n\n    when n < 5 then m :: 9\n    return m\n");
}

/// The joined local spans both branches: `m` is 1 or 8 afterwards, no more and no less.
#[test]
fn the_joined_local_spans_exactly_the_branches() {
    let body = "    m in 0 to 9: 0\n    when n < 5 then m :: 1\n    else m :: 8\n    return m\n";
    ok(&format!("f(n in 0 to 9) in 1 to 8:\n{body}"));
    refused(&format!("f(n in 0 to 9) in 2 to 8:\n{body}"));
    refused(&format!("f(n in 0 to 9) in 1 to 7:\n{body}"));
}

/// With no `else`, the empty path keeps the value `m` had.
#[test]
fn a_missing_else_keeps_the_old_value() {
    let body = "    m in 0 to 9: 3\n    when n < 5 then m :: 7\n    return m\n";
    ok(&format!("f(n in 0 to 9) in 3 to 7:\n{body}"));
    refused(&format!("f(n in 0 to 9) in 4 to 7:\n{body}"));
}

/// The comparison is not a fact after the statement: `n - 5` can still be negative.
#[test]
fn the_guard_does_not_survive_the_join() {
    refused(
        "f(n in 0 to 9) in 0 to 4:\n    m in 0 to 9: 0\n    when n < 5 then m :: 1\n    return n - 5\n",
    );
}

/// When only one path is possible it keeps what it learned, as a sole surviving path does.
#[test]
fn a_decided_condition_keeps_its_branch() {
    ok("f(n in 0 to 9) in 1 to 1:\n    m in 0 to 9: 0\n    when n < 10 then m :: 1\n    return m\n");
    ok("f(n in 0 to 9) in 2 to 2:\n    m in 0 to 9: 0\n    when n > 20 then m :: 1\n    else m :: 2\n    return m\n");
}

#[test]
fn a_local_bound_inside_a_branch_ends_with_it() {
    refused("f(n in 0 to 9) in 0 to 9:\n    when n < 5 then\n        t: 1\n    return t\n");
    ok("f(n in 0 to 9) in 0 to 9:\n    m in 0 to 9: 0\n    when n < 5 then\n        t: 1\n        m :: t\n    return m\n");
}

#[test]
fn a_branch_may_hold_another_conditional() {
    let source = "\
f(n in 0 to 9) in 0 to 3:
    m in 0 to 9: 0
    when n < 5 then
        m :: 1
        when n < 2 then m :: 3
    return m
";
    ok(source);
}

/// A block that ends in `return` is a guard, and valid. One that returns from inside a nested
/// guard and then falls through is not supported.
#[test]
fn a_branch_that_falls_through_may_not_return_early() {
    ok("f(n in 0 to 9) in 0 to 9:\n    m: 0\n    when n < 5 then\n        return 1\n    return m\n");
    refused("f(n in 0 to 9) in 0 to 9:\n    m in 0 to 9: 0\n    when n < 5 then\n        when n < 2 then return 1\n        m :: 2\n    return m\n");
}

/// Regression: a literal has no identity, so a local reassigned from one literal to another
/// must still count as changed. Here `m` is 0 or 6, never within `0 to 5`.
#[test]
fn a_reassigned_literal_is_a_changed_local() {
    refused("f(n in 0 to 9) in 0 to 5:\n    m in 0 to 9: 0\n    when n > 6 then m :: 6\n    return m\n");
    ok("f(n in 0 to 9) in 0 to 6:\n    m in 0 to 9: 0\n    when n > 6 then m :: 6\n    return m\n");
}

/// The point of the join: thirty statements in a row stay linear. Enumerating them would be
/// a billion worlds.
#[test]
fn thirty_sequential_conditionals_stay_linear() {
    let mut source = String::from("f(n in 0 to 99) in 0 to 99:\n    m in 0 to 99: 0\n");
    for k in 0..30 {
        source.push_str(&format!("    when n = {k} then m :: {k}\n"));
    }
    source.push_str("    return m\n");
    ok(&source);
}

/// Each statement that reassigns the same local joins with the one before it.
#[test]
fn a_chain_of_conditionals_accumulates_the_span() {
    let source = "\
f(n in 0 to 9) in 0 to 6:
    m in 0 to 9: 0
    when n < 3 then m :: 2
    when n > 6 then m :: 6
    return m
";
    ok(source);
    let tight = source.replace("in 0 to 6:", "in 0 to 5:");
    refused(&tight);
}

/// A guard that returns still works beside a fall-through conditional.
#[test]
fn guards_and_joins_mix() {
    let source = "\
f(n in 0 to 9) in 0 to 9:
    when n = 0 then return 9
    m in 0 to 9: n
    when n > 5 then m :: 5
    return m
";
    ok(source);
}

fn recorded(text: &str, function: &str) -> Option<Vec<typecheck::CheckedSourceBodyKind>> {
    let module = check(text).expect("checks");
    let file = module.checked();
    let index = file.functions.iter().position(|f| f.name == function)?;
    let body = file.bodies[index].as_ref()?;
    Some(body.expressions.iter().map(|e| e.kind.clone()).collect())
}

/// Each local the branches changed is a `Select` of its two values in the recorded body.
#[test]
fn a_joined_local_is_recorded_as_a_select() {
    let kinds = recorded(
        "f(n in 0 to 9) in 0 to 9:\n    m in 0 to 9: 0\n    when n < 5 then m :: 7\n    return m\n",
        "f",
    )
    .expect("recorded");
    let selects = kinds
        .iter()
        .filter(|kind| matches!(kind, typecheck::CheckedSourceBodyKind::Select { .. }))
        .count();
    assert_eq!(selects, 1, "{kinds:?}");
}

/// A known call folds through the join.
#[test]
fn a_known_call_folds_through_a_join() {
    let text = "f(n in 0 to 9) in 0 to 9:\n    m in 0 to 9: 0\n    when n < 5 then m :: 7\n    return m\n\ng() in 0 to 9: return f(1)\n\nh() in 0 to 9: return f(8)\n";
    let module = check(text).expect("checks");
    let file = module.checked();
    for (name, expected) in [("g", 7), ("h", 0)] {
        let index = file.functions.iter().position(|f| f.name == name).unwrap();
        let body = file.bodies[index].as_ref().expect("recorded");
        assert!(
            body.expressions.iter().all(|e| !matches!(
                e.kind,
                typecheck::CheckedSourceBodyKind::Call { .. }
            )),
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

/// A call in a branch checks, but is not recorded: both branches are computed, and the call
/// must not run when its branch is not taken.
#[test]
fn a_call_inside_a_branch_is_not_recorded() {
    let text = "same(x in 0 to 9) in 0 to 9: return x\n\nf(n in 0 to 9) in 0 to 9:\n    m in 0 to 9: 0\n    when n < 5 then m :: same(n)\n    return m\n";
    assert!(recorded(text, "f").is_none());
    // A call that folds to a constant leaves nothing to run, so it is recorded.
    let folded = "five() in 0 to 9: return 5\n\nf(n in 0 to 9) in 0 to 9:\n    m in 0 to 9: 0\n    when n < 5 then m :: five()\n    return m\n";
    assert!(recorded(folded, "f").is_some());
}
