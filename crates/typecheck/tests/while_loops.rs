//! `while left <comparison> right do body`.
//!
//! The body is checked once, at a head state where everything it could change is forgotten down
//! to its declared domain. After the loop the head state holds again with the condition false.
//! Nothing is inferred: what a pass establishes is lost unless a declared domain says it.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.loops", &source, &[], Limits::default())
}

fn ok(text: &str) {
    let checked = check(text);
    assert!(checked.is_ok(), "{text}\n{:?}", checked.err());
}

fn refused(text: &str) {
    assert!(check(text).is_err(), "{text}");
}

#[test]
fn a_counting_loop_checks() {
    ok(
        "f() in 0 to 10:\n    i in 0 to 10: 0\n    while i < 10 do\n        i :: i + 1\n    return i\n",
    );
}

/// After the loop the condition is false and the declared domain holds: `i` is exactly 10 only
/// if the checker knew it, which it does not (nothing is inferred), so the result must allow
/// everything the domain does.
#[test]
fn only_the_declared_domain_and_the_negated_condition_survive() {
    let body = "    i in 0 to 10: 0\n    while i < 10 do\n        i :: i + 1\n    return i\n";
    // i >= 10 holds after the loop and i is at most 10, so this is exact.
    ok(&format!("f() in 10 to 10:\n{body}"));
    // Nothing is claimed beyond that.
    refused(&format!("f() in 11 to 11:\n{body}"));
}

/// The body is checked against the declared domain, not the entry value: the assignment `i + 1`
/// must fit `0 to 10` for every `i` the head allows with `i < 20`.
#[test]
fn a_pass_that_can_leave_the_domain_is_refused() {
    refused(
        "f() in 0 to 10:\n    i in 0 to 10: 0\n    while i < 20 do\n        i :: i + 1\n    return i\n",
    );
}

/// Inside the body the condition holds: `i < 5` makes `i + 1` fit `0 to 5`.
#[test]
fn the_condition_holds_inside_the_body() {
    ok(
        "f() in 0 to 5:\n    i in 0 to 5: 0\n    while i < 5 do\n        i :: i + 1\n    return i\n",
    );
    refused(
        "f() in 0 to 5:\n    i in 0 to 5: 0\n    while i < 6 do\n        i :: i + 1\n    return i\n",
    );
}

/// A condition that cannot hold at the head cannot hold on entry either (the head spans the
/// entry state), so the loop never runs and the state is the entry state.
#[test]
fn a_loop_that_cannot_start_leaves_the_entry_state() {
    ok(
        "f() in 3 to 3:\n    i in 0 to 10: 3\n    while i > 10 do\n        i :: i + 1\n    return i\n",
    );
}

/// A condition that can hold at the head might not hold on entry, but the loop is still
/// analysed, and afterwards only the head state and the failed condition are known.
#[test]
fn a_condition_possible_at_the_head_is_analysed() {
    ok(
        "f() in 0 to 10:\n    i in 0 to 10: 3\n    while i > 8 do\n        i :: i - 1\n    return i\n",
    );
}

#[test]
fn a_fall_through_conditional_may_sit_in_the_body() {
    let source = "\
f() in 0 to 10:
    i in 0 to 10: 0
    j in 0 to 10: 0
    while i < 10 do
        when i < 5 then
            j :: i
        i :: i + 1
    return i
";
    ok(source);
}

/// Each loop forgets only what its body can change; two loops in a row are two heads.
#[test]
fn loops_in_sequence_each_have_their_own_head() {
    let source = "\
f() in 0 to 10:
    i in 0 to 10: 0
    while i < 10 do
        i :: i + 1
    j in 0 to 10: 0
    while j < i do
        j :: j + 1
    return j
";
    ok(source);
}

#[test]
fn a_loop_may_sit_in_another_loop() {
    let source = "\
f() in 0 to 10:
    i in 0 to 10: 0
    while i < 10 do
        j in 0 to 10: 0
        while j < i do
            j :: j + 1
        i :: i + 1
    return i
";
    ok(source);
}

/// A `return` inside a loop over a sequence is still an exit that loop cannot hold: it consumes
/// its elements one by one and a return would leave some unconsumed.
#[test]
fn a_return_inside_a_loop_over_a_sequence_is_not_supported() {
    let source = "#unrestricted\nToken has\n    number in 0 to 9\n\nf(values (Token, Token)) in 0 to 9:\n    for value in values do\n        when value.number = 5 then return 5\n    return 0\n";
    assert!(matches!(check(source), Err(SourceCheckError::Unsupported)));
}

/// Sixteen loops in a row check: each is one head, never a product of worlds. (Cost per loop
/// does grow, because every head leaves facts and identities behind; about thirty in one
/// function reach the default budget. Collecting what no name refers to is future work.)
#[test]
fn many_loops_in_a_row_check() {
    let mut source = String::from("f() in 0 to 10:\n    i in 0 to 10: 0\n");
    for _ in 0..16 {
        source.push_str("    while i < 10 do\n        i :: i + 1\n");
    }
    source.push_str("    return i\n");
    ok(&source);
}

/// Soundness of the modified set: `j` is assigned only inside a conditional in the body, and must
/// still be forgotten at the head. If it kept its entry value 0, `j + 1` would look safe.
#[test]
fn a_local_assigned_in_a_nested_conditional_is_forgotten() {
    refused(
        "f() in 0 to 10:\n    i in 0 to 10: 0\n    j in 0 to 10: 0\n    while i < 10 do\n        when i < 5 then\n            j :: j + 1\n        i :: i + 1\n    return j\n",
    );
}

/// Precision of the modified set: `limit` is never assigned, so the fact the outer loop gave it
/// (`limit < 10`) survives the inner loop, and the later `limit + 1` still fits.
#[test]
fn a_local_the_loop_cannot_change_keeps_its_facts() {
    ok(
        "f() in 0 to 10:\n    i in 0 to 10: 0\n    limit in 0 to 10: 0\n    while limit < 10 do\n        while i < 10 do\n            i :: i + 1\n        limit :: limit + 1\n    return limit\n",
    );
}

// A `return` inside a loop ends the path with the returned value.
// The returned value is typed against the function's result, owners are accounted for at the
// return, and the loop's other paths go on as before.

#[test]
fn a_guard_that_returns_may_sit_in_a_while_body() {
    let source = "\
f(n in 0 to 9) in 0 to 9:
    i in 0 to 10: 0
    while i < 10 do
        when i = n then return i
        i :: i + 1
    return 0
";
    ok(source);
}

/// The returned value is checked against the function's result, not the loop's unit.
#[test]
fn a_returned_value_outside_the_result_is_refused() {
    refused(
        "f(n in 0 to 9) in 0 to 4:\n    i in 0 to 10: 0\n    while i < 10 do\n        when i = n then return i\n        i :: i + 1\n    return 0\n",
    );
}

/// Where the guard holds, `i` equals `n`, so the value returned is within `n`'s domain.
#[test]
fn the_guard_holds_inside_the_returning_block_of_a_loop() {
    ok(
        "f(n in 0 to 4) in 0 to 4:\n    i in 0 to 10: 0\n    while i < 10 do\n        when i = n then return i\n        i :: i + 1\n    return 0\n",
    );
}

/// Only the returning path ends: the others reach the back edge, where the body must still
/// keep the head's domains.
#[test]
fn the_paths_that_do_not_return_still_obey_the_loop() {
    refused(
        "f(n in 0 to 9) in 0 to 9:\n    i in 0 to 10: 0\n    while i < 20 do\n        when i = n then return i\n        i :: i + 1\n    return 0\n",
    );
}

#[test]
fn a_return_may_follow_other_statements_in_the_loop() {
    ok(
        "f(n in 0 to 9) in 0 to 9:\n    i in 0 to 10: 0\n    while i < 10 do\n        j in 0 to 10: i\n        when j = n then\n            return j\n        i :: i + 1\n    return 0\n",
    );
}

#[test]
fn a_plain_return_ends_a_loop_body() {
    ok(
        "f(n in 0 to 9) in 0 to 9:\n    i in 0 to 10: 0\n    while i < 10 do\n        return i\n    return 0\n",
    );
}

#[test]
fn a_return_in_a_possible_while_loop_does_not_hide_the_skipped_path() {
    let source =
        "f(flag in 0 to 1) in 0 to 0:\n    while flag = 1 do\n        return 0\n    return 1\n";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Type);
    ok(&source.replace("return 1\n", "return 0\n"));
    ok(&source.replace("flag in 0 to 1", "flag in 1 to 1"));
    refused(&source.replace("flag in 0 to 1", "flag in 0 to 0"));
    let invariant = source.replace("flag = 1 do", "flag = 1 where flag >= 0 do");
    refused(&invariant);
    ok(&invariant.replace("return 1\n", "return 0\n"));
}

#[test]
fn a_range_loop_may_return() {
    ok(
        "f(n in 0 to 9) in 0 to 9:\n    for k in 0 to 9 do\n        when k = n then return k\n    return 0\n",
    );
    refused(
        "f(n in 0 to 9) in 0 to 3:\n    for k in 0 to 9 do\n        when k = n then return k\n    return 0\n",
    );
}

#[test]
fn a_return_inside_nested_loops_ends_the_whole_path() {
    let source = "\
f(n in 0 to 9) in 0 to 9:
    i in 0 to 10: 0
    while i < 10 do
        j in 0 to 10: 0
        while j < 10 do
            when j = n then return j
            j :: j + 1
        i :: i + 1
    return 0
";
    ok(source);
}

/// Each returning test in a loop body is one more path, not a doubling.
#[test]
fn several_returns_in_one_body_check() {
    ok(
        "f(n in 0 to 9) in 0 to 9:\n    i in 0 to 10: 0\n    while i < 10 do\n        when i = 1 then return 1\n        when i = 2 then return 2\n        when i = 3 then return 3\n        when i = n then return i\n        i :: i + 1\n    return 0\n",
    );
}

/// A return accounts for every live owner at that point, on its own path. Returning from the
/// loop with `t` still unconsumed is refused...
#[test]
fn a_return_in_a_loop_must_end_the_owners_live_there() {
    let source = "\
Token has
    number in 0 to 9

f(n in 0 to 9) in 0 to 9:
    t := Token(number: 1)
    i in 0 to 10: 0
    while i < 10 do
        when i = n then return 1
        i :: i + 1
    Token(number) := t
    return number
";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Unconsumed);
}

/// ...while ending `t` on the returning path alone is fine: the paths that go on keep it, and the
/// code after the loop consumes it.
#[test]
fn ending_the_owner_on_the_returning_path_alone_is_accepted() {
    let source = "\
Token has
    number in 0 to 9

f(n in 0 to 9) in 0 to 9:
    t := Token(number: 1)
    i in 0 to 10: 0
    while i < 10 do
        when i = n then
            Token(number) := t
            return number
        i :: i + 1
    Token(number) := t
    return number
";
    ok(source);
}

#[test]
fn a_skipped_returning_while_keeps_its_linear_cleanup_obligation() {
    let source = "\
Token has number in 0 to 9
f(flag in 0 to 1) in 0 to 9:
    token := Token(number: 1)
    while flag = 1 do
        Token(number) := token
        return number
    return 0
";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Unconsumed);
    ok(&source.replace(
        "    return 0\n",
        "    Token(number) := token\n    return number\n",
    ));
    ok(&source.replace("flag in 0 to 1", "flag in 1 to 1"));
}

#[test]
fn a_skipped_returning_while_must_establish_the_mut_postcondition() {
    let source = "\
f(flag in 0 to 1, x mut in 0 to 9) where x :: 0:
    while flag > 0 do
        x :: 0
        return
    return
";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Unproved);
    ok(&source.replace("\n    return\n", "\n    x :: 0\n    return\n"));
}

#[test]
fn returning_while_worlds_keep_their_condition_facts_for_callers() {
    ok("\
f(flag in 0 to 1) in 0 to 1:
    while flag > 0 do
        return 1
    return flag
skipped() in 0 to 0: return f(0)
entered() in 1 to 1: return f(1)
");
    ok("\
f(a in 0 to 1, b in 0 to 1) in 0 to 0:
    while a > 0 or b > 0 do
        return 0
    return a + b
");
    refused(
        "\
f(a in 0 to 1, b in 0 to 1) in 0 to 0:
    while a > 0 and b > 0 do
        return 0
    return 1
",
    );
}
