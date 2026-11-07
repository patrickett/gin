//! `when attempt is Held(h) then ...` takes the payload of the `Held` variant out of `attempt`.
//! (Narrowing `attempt` itself to a type member was withdrawn with type members: a variant is the
//! union's own constructor, and a payload variant is matched through its wrapper.) These tests
//! keep the properties that still hold: the branch owns the payload and must consume it, the
//! code after a one-armed case that returns sees only the other variants, and a private record
//! can be passed along but not opened through the pattern.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_file, check_module};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default()).map(|_| ())
}

const TYPES: &str = "\
Held has
    n in 0 to 9
    end(self):
        Held(n) := self
        return
    count(self ref) in 0 to 9: return self.n

Attempt is Held(Held) or Lost

make() Attempt: return Held(Held(n: 3))
take_held(h Held):
    h.end
    return

";

fn with(tail: &str) -> String {
    format!("{TYPES}{tail}\nmain(): return\n")
}

#[test]
fn the_branch_sees_the_member_and_consumes_the_original() {
    let ok = with("run(r Attempt):\n    when r is Held(h) then\n        take_held(h)\n        return\n    return");
    assert_eq!(check(&ok), Ok(()));
    // A member method is available inside the branch.
    let method = with("run(r Attempt) in 0 to 9:\n    when r is Held(h) then\n        n: h.count\n        take_held(h)\n        return n\n    return 0");
    assert_eq!(check(&method), Ok(()));
}

#[test]
fn the_else_side_is_the_other_member() {
    // After the one-armed case returns, `r` is `Lost`, which may be dropped: nothing to end.
    let ok = with("run(r Attempt):\n    when r is Held(h) then\n        take_held(h)\n        return\n    return");
    assert_eq!(check(&ok), Ok(()));
    // Not a `Held`, the rest of the body cannot pass `r` on as one.
    let wrong = with("run(r Attempt):\n    when r is Held(h) then\n        take_held(h)\n        return\n    take_held(r)\n    return");
    assert!(check(&wrong).is_err());
}

#[test]
fn a_member_only_use_outside_the_branch_is_refused() {
    // `r` is any member here: passing it where a `Held` is wanted is refused.
    let outside = with("run(r Attempt):\n    take_held(r)\n    return");
    assert!(check(&outside).is_err());
}

#[test]
fn a_narrowed_linear_value_is_consumed_once() {
    let twice = with("run(r Attempt):\n    when r is Held(h) then\n        take_held(h)\n        take_held(h)\n        return\n    return");
    assert_eq!(check(&twice), Err(SourceCheckError::UseAfterMove));
    // Not consuming it in the branch leaves a value that must be ended.
    let kept = with("run(r Attempt):\n    when r is Held(h) then\n        return\n    return");
    assert_eq!(check(&kept), Err(SourceCheckError::Unconsumed));
}

fn module(name: &str, text: &str, deps: &[&CheckedSourceModule]) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict fixture");
    check_module(name, &source, deps, Limits::default())
}

#[test]
fn another_modules_private_record_is_narrowed_but_not_opened() {
    let library = module(
        "lib.span",
        "Secret has\n    #private\n    n in 0 to 9\n    end(self):\n        Secret(n) := self\n        return\n\nMaybe is Secret(Secret) or None\n\nfind() Maybe: return Secret(Secret(n: 1))\n",
        &[],
    )
    .unwrap();
    // Narrowing needs no access to the representation.
    let narrow = "use lib.span Secret, Maybe, find\nrun():\n    m: find()\n    when m is Secret(s) then\n        s.end\n        return\n    return\n";
    assert!(module("app", narrow, &[&library]).is_ok(), "{:?}", module("app", narrow, &[&library]).err());
    // Taking it apart, or reading a field, still does.
    let open = "use lib.span Secret, Maybe, find\nrun():\n    m: find()\n    when m is Secret(s) then\n        Secret(n) := s\n        return\n    return\n";
    assert_eq!(module("app", open, &[&library]).map(|_| ()), Err(SourceCheckError::Privacy));
    let read = "use lib.span Secret, Maybe, find\nrun() in 0 to 9:\n    m: find()\n    when m is Secret(s) then\n        n: s.n\n        s.end\n        return n\n    return 0\n";
    assert_eq!(module("app", read, &[&library]).map(|_| ()), Err(SourceCheckError::Privacy));
}

#[test]
fn narrowing_is_refused_under_and_or_and_chains_until_each_arm_can_be_proved_sound() {
    // Narrowing is built for the one-armed statement and the inline `when x is M then a else b`
    // only. A member test combined with another condition, or with `or`, would have to say which
    // world each arm is in (an `or` of two members narrows to neither); until then it is refused,
    // never silently treated as narrowing.
    for body in [
        "run(r Attempt, k in 0 to 9):\n    when r is Held(h) and k < 5 then\n        take_held(h)\n        return\n    return",
        "run(r Attempt):\n    when r is Held(h) or r is Lost then\n        return\n    return",
        "run(r Attempt, q Attempt):\n    when r is Held(h) and q is Held(g) then\n        take_held(h)\n        take_held(g)\n        return\n    return",
    ] {
        let source = with(body);
        assert!(check(&source).is_err(), "{source}");
    }
    // The supported shape right next to them is accepted (the control).
    let control = with("run(r Attempt):\n    when r is Held(h) then\n        take_held(h)\n        return\n    return");
    assert_eq!(check(&control), Ok(()));
}
