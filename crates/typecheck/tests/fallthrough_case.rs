//! A one-armed `when subject is Member then <block>` whose block does not return: the world where
//! the subject is the member runs the block and goes on, every other world passes it over.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let file = parser::parse_strict(text).unwrap_or_else(|d| panic!("{text}\n{d:?}"));
    check_file(&file, Limits::default()).map(|_| ())
}

const BOOL: &str = "#unrestricted\nBool is True or False\n\n";

#[test]
fn an_arm_that_falls_through_checks_and_the_body_goes_on() {
    let source = format!(
        "{BOOL}run(flag Bool) in 0 to 9:\n    count in 0 to 9: 0\n    when flag is True then\n        count :: 5\n    return count\n"
    );
    assert_eq!(check(&source), Ok(()));
}

#[test]
fn it_may_be_the_last_statement() {
    let source = format!("{BOOL}run(flag Bool):\n    count in 0 to 9: 0\n    when flag is True then\n        count :: 5\n");
    assert_eq!(check(&source), Ok(()));
}

#[test]
fn a_fact_established_only_in_the_arm_is_not_assumed_afterwards() {
    // In the world where the flag is False the count is still 0.
    let source = format!(
        "{BOOL}run(flag Bool) in 5 to 5:\n    count in 0 to 9: 0\n    when flag is True then\n        count :: 5\n    return count\n"
    );
    assert!(check(&source).is_err(), "{:?}", check(&source));
    // Both worlds agree when both set it.
    let both = format!(
        "{BOOL}run(flag Bool) in 5 to 5:\n    count in 0 to 9: 5\n    when flag is True then\n        count :: 5\n    return count\n"
    );
    assert_eq!(check(&both), Ok(()));
}

const RESOURCE: &str = "\
#unrestricted
Bool is True or False

Token has
    n in 0 to 9

    release(self):
        Token(n) := self
        return

";

#[test]
fn a_linear_value_consumed_in_the_arm_only_is_refused_as_unconsumed() {
    let source = format!(
        "{RESOURCE}run(flag Bool):\n    t: Token(n: 1)\n    when flag is True then\n        t.release\n    return\n"
    );
    assert_eq!(check(&source), Err(SourceCheckError::Unconsumed), "{:?}", check(&source));
    // Control: ended on both paths, it is accepted.
    let both = format!(
        "{RESOURCE}run(flag Bool):\n    t: Token(n: 1)\n    when flag is True then\n        t.release\n    when flag is False then\n        t.release\n    return\n"
    );
    assert_eq!(check(&both), Ok(()), "{:?}", check(&both));
}

const OUTCOME: &str = "\
#unrestricted
Code is in 0 to 9

#unrestricted
Outcome is Fine or Failed(Code)

rank(o Outcome) in 0 to 1: when o is
    Fine then 0
    Failed(_) then 1

";

#[test]
fn narrowing_to_the_member_applies_only_inside_the_arm() {
    // Inside the arm the subject is the member; after it, it is the whole sum again.
    let source = format!(
        "{OUTCOME}run(o Outcome) in 0 to 1:\n    count in 0 to 1: 0\n    when o is Failed(code) then\n        count :: 1\n    return rank(o)\n"
    );
    assert_eq!(check(&source), Ok(()), "{:?}", check(&source));
    // Negative control: a binder of the arm is not in scope after it.
    let leaked = format!(
        "{OUTCOME}run(o Outcome) in 0 to 9:\n    when o is Failed(code) then\n        return 1\n    return code\n"
    );
    assert!(check(&leaked).is_err());
}

#[test]
fn an_arm_that_returns_keeps_its_old_meaning() {
    let source = format!("{OUTCOME}run(o Outcome) in 0 to 1:\n    when o is Failed(_) then\n        return 1\n    return 0\n");
    assert_eq!(check(&source), Ok(()));
    // A guard that returns still needs something after it at the start of a function.
    let last = format!("{OUTCOME}run(o Outcome) in 0 to 1:\n    when o is Failed(_) then\n        return 1\n");
    assert!(check(&last).is_err());
}

#[test]
fn an_else_branch_is_still_not_supported() {
    let source = format!("{BOOL}run(flag Bool):\n    when flag is True then\n        return\n    else\n        return\n");
    assert!(check(&source).is_err());
}
