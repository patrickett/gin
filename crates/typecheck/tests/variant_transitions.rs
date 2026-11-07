//! `::` transitions in a variant's `where` (decision 9): proved where the variant is returned,
//! and learned by a caller where it matches that variant. The place is forgotten at the call.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_file_diagnostic, check_module};

fn run(text: &str) -> Result<(), SourceCheckError> {
    let file = parser::parse_strict(text).unwrap_or_else(|d| panic!("{text}\n{d:?}"));
    check_file_diagnostic(&file, Limits::default()).map(|_| ()).map_err(|failure| failure.error)
}

const COUNTER: &str = "\
#unrestricted
Counter has
    n in 0 to 100

";

const BUMP: &str = "\
bump(c mut Counter and c.n < 100) Grown where (c.n :: c.n + 1) or Kept where (c.n :: c.n):
    when c.n < 50 then
        c.n :: c.n + 1
        return Grown
    return Kept

";

fn with(tail: &str) -> String {
    format!("{COUNTER}{BUMP}{tail}")
}

const CALLER: &str = "\
use_it(c mut Counter and c.n = 3) in 3 to 4: return when bump(c) is
    Grown then c.n
    Kept then c.n
";

#[test]
fn each_variant_proves_its_own_transition_and_the_caller_learns_it_in_the_arm() {
    assert_eq!(run(&with(CALLER)), Ok(()));
    // Negative control: without the transitions the caller knows nothing of `c.n`.
    let bare = with(CALLER).replace(" where (c.n :: c.n + 1) or Kept where (c.n :: c.n):", " or Kept:");
    assert!(run(&bare).is_err());
    // The arm learns its own variant's value, not the other's.
    let tight = with(CALLER).replace("in 3 to 4: return", "in 4 to 4: return");
    assert!(run(&tight).is_err());
}

#[test]
fn a_transition_in_one_variant_only() {
    let caller = "use_it(c mut Counter and c.n = 3) in 0 to 100: return when bump(c) is\n    Grown then c.n - 4\n    Kept then c.n\n";
    let text = format!("{COUNTER}{}{caller}", BUMP.replace(" or Kept where (c.n :: c.n):", " or Kept:"));
    assert_eq!(run(&text), Ok(()));
    // In `Kept` nothing is learned: `c.n - 4` there is not proved.
    let wrong = text.replace("    Kept then c.n\n", "    Kept then c.n - 4\n");
    assert!(run(&wrong).is_err());
}

#[test]
fn a_transition_false_where_its_variant_is_returned_is_unproved() {
    let lying = with("").replace("        c.n :: c.n + 1\n        return Grown", "        c.n :: c.n + 2\n        return Grown");
    assert_eq!(run(&lying), Err(SourceCheckError::Unproved));
    // The variants' transitions are not swapped: `Kept` with a changed place is refused too.
    let changed = with("").replace("    return Kept", "    c.n :: 0\n    return Kept");
    assert_eq!(run(&changed), Err(SourceCheckError::Unproved));
}

#[test]
fn a_place_used_before_the_result_is_matched_stays_forgotten() {
    let early = with(
        "use_it(c mut Counter and c.n = 3) in 3 to 4:\n    r: bump(c)\n    seen in 3 to 4: c.n\n    return when r is\n        Grown then seen\n        Kept then seen\n",
    );
    assert!(run(&early).is_err());
    let late = with(
        "use_it(c mut Counter and c.n = 3) in 3 to 4:\n    r: bump(c)\n    return when r is\n        Grown then c.n\n        Kept then c.n\n",
    );
    assert_eq!(run(&late), Ok(()));
}

#[test]
fn a_transitioned_place_in_a_loop() {
    let looped = with(
        "climb(c mut Counter and c.n = 0) in 0 to 10:\n    for i in 0 to 9 where c.n <= i do\n        step in 0 to 1: when bump(c) is\n            Grown then 1\n            Kept then 0\n    return c.n\n",
    );
    assert_eq!(run(&looped), Ok(()));
    // Negative control: the invariant bounds `c.n` by the passes, not tighter.
    assert!(run(&looped.replace("in 0 to 10:", "in 0 to 9:")).is_err());
}

#[test]
fn transitions_reach_a_caller_in_another_module() {
    let library_text = format!("{COUNTER}{BUMP}");
    let library_file = parser::parse_strict(&library_text).unwrap();
    let library: CheckedSourceModule = check_module("lib.counter", &library_file, &[], Limits::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let client = format!("use lib.counter Counter, bump\n\n{CALLER}");
    let file = parser::parse_strict(&client).unwrap();
    check_module("app.client", &file, &[&library], Limits::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let tight = client.replace("in 3 to 4: return", "in 4 to 4: return");
    let file = parser::parse_strict(&tight).unwrap();
    assert!(check_module("app.client", &file, &[&library], Limits::default()).is_err());
}

#[test]
fn a_trait_provided_inline_result_carries_its_transitions() {
    let text = format!(
        "{COUNTER}Stepper has
    step(self, c mut Counter and c.n < 100) Grown where (c.n :: c.n + 1) or Kept where (c.n :: c.n)

#unrestricted
Plain has Stepper
    k in 0 to 9

    step(self, c mut Counter and c.n < 100) Grown where (c.n :: c.n + 1) or Kept where (c.n :: c.n):
        when c.n < 50 then
            c.n :: c.n + 1
            return Grown
        return Kept

rank(kind has Stepper, s kind, c mut Counter and c.n = 3) in 3 to 4: return when s.step(c) is
    Grown then c.n
    Kept then c.n

direct(s Plain, c mut Counter and c.n = 3) in 3 to 4: return when s.step(c) is
    Grown then c.n
    Kept then c.n
"
    );
    assert_eq!(run(&text), Ok(()));
    // The provider must repeat the transition text.
    let differs = text.replacen("    step(self, c mut Counter and c.n < 100) Grown where (c.n :: c.n + 1) or Kept where (c.n :: c.n):", "    step(self, c mut Counter and c.n < 100) Grown where (c.n :: c.n + 1) or Kept:", 1);
    assert_eq!(run(&differs), Err(SourceCheckError::Type));
    // Through the bound, the arm learns its own variant's value only.
    let tight = text.replace("rank(kind has Stepper, s kind, c mut Counter and c.n = 3) in 3 to 4:", "rank(kind has Stepper, s kind, c mut Counter and c.n = 3) in 4 to 4:");
    assert!(run(&tight).is_err());
}
