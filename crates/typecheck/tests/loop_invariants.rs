//! `while cond where P do` and `for i in a to b where P do`: the invariant is optional and never
//! inferred. It must hold on entry, is assumed at the head of every pass, must be re-established
//! by the end of the pass, and at exit it (with the negated condition, or the counter just past
//! the end) is how the checker learns the loop's total effect on a record.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default())
}

const BUF: &str = "\
Buf has
    n in 0 to 100
    len(self ref) in 0 to 100: return self.n
    add(self mut and self.n < 100) where self.n :: self.n + 1:
        self.n :: self.n + 1
        return
";

fn with_while(head: &str) -> String {
    format!("{BUF}fill(b mut Buf and b.len = 0, k in 0 to 40) where b.len :: k:\n    i in 0 to 50: 0\n    while i < k{head} do\n        b.add\n        i :: i + 1\n    return\n")
}

fn with_for(head: &str) -> String {
    format!("{BUF}fill(b mut Buf and b.len = 0, k in 1 to 50) where b.len :: k:\n    for i in 0 to k - 1{head} do\n        b.add\n    return\n")
}

#[test]
fn a_while_invariant_gives_the_loops_total_effect() {
    let source = with_while(" where b.len = i and i <= k");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn a_range_for_invariant_gives_the_loops_total_effect() {
    let source = with_for(" where b.len = i");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn an_invariant_false_at_entry_is_refused() {
    assert_eq!(check(&with_while(" where b.len = i + 1 and i <= k")), Err(SourceCheckError::Unproved));
    assert_eq!(check(&with_for(" where b.len = i + 1")), Err(SourceCheckError::Unproved));
}

#[test]
fn a_body_that_does_not_reestablish_it_is_refused() {
    let source = with_while(" where b.len = i and i <= k").replace("i :: i + 1", "i :: i + 2");
    assert_eq!(check(&source), Err(SourceCheckError::Unproved));
    let source = with_for(" where b.len = i").replace("        b.add\n", "        b.add\n        b.add\n");
    assert_eq!(check(&source), Err(SourceCheckError::Unproved));
}

#[test]
fn an_invariant_too_weak_for_the_claim_after_the_loop_is_refused_at_the_use() {
    assert_eq!(check(&with_while(" where b.len = i")), Err(SourceCheckError::Unproved));
    assert_eq!(check(&with_for(" where i >= 0")), Err(SourceCheckError::Unproved));
}

#[test]
fn a_push_that_needs_a_moving_bound_is_unproved_without_an_invariant() {
    assert_eq!(check(&with_while("")), Err(SourceCheckError::Unproved));
    assert_eq!(check(&with_for("")), Err(SourceCheckError::Unproved));
}

#[test]
fn the_wrong_direction_and_a_counter_not_matching_the_bound_are_refused() {
    assert_eq!(check(&with_while(" where b.len + i = k and i <= k")), Err(SourceCheckError::Unproved));
    assert_eq!(check(&with_for(" where b.len + i = 0")), Err(SourceCheckError::Unproved));
    // The range is inclusive: `0 to k` makes `k + 1` passes, so the claim `b.len :: k` is false.
    let source = with_for(" where b.len = i").replace("to k - 1", "to k");
    assert_eq!(check(&source), Err(SourceCheckError::Unproved));
}

#[test]
fn a_record_changed_by_a_bare_method_call_is_forgotten_at_the_loop_head() {
    // `b.add` written without parentheses lends `b` `mut`: what the entry claim said of `b.len`
    // is not true of later passes, so reading it as 0 inside the loop is refused.
    let source = format!("{BUF}fill(b mut Buf and b.len = 0, k in 0 to 40):\n    i in 0 to 50: 0\n    while i < 3 do\n        z in 0 to 0: b.len\n        b.add\n        i :: i + 1\n    return\n");
    assert_eq!(check(&source), Err(SourceCheckError::Type));
    // Control: before the loop the claim holds.
    let before = format!("{BUF}fill(b mut Buf and b.len = 0, k in 0 to 40):\n    z in 0 to 0: b.len\n    return\n");
    assert!(check(&before).is_ok(), "{:?}", check(&before));
}

#[test]
fn an_invariant_that_allows_overshoot_does_not_prove_the_exact_total() {
    // Two pushes per pass: `b.len = i` holds, but the loop can end one past `k`.
    let source = format!("{BUF}fill(b mut Buf and b.len = 0, k in 0 to 40) where b.len :: k:\n    i in 0 to 50: 0\n    while i < k where b.len = i do\n        b.add\n        b.add\n        i :: i + 2\n    return\n");
    assert_eq!(check(&source), Err(SourceCheckError::Unproved));
}
