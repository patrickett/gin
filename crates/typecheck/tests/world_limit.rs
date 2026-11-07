//! A function whose callee outcomes multiply past the cap is refused by name.
use typecheck::{Limits, SourceCheckError, check_module_files_diagnostic};

const OUTCOME: &str = "\
#unrestricted
Code is in 0 to 9

#unrestricted
Outcome is Done or Failure(Code)

attempt(n in 0 to 9) Outcome:
    when n < 5 then return Outcome.Done
    return Failure(n as Code)
";

/// `run` calls `attempt` `calls` times in a row and discards every result: 2^calls worlds.
fn source(calls: usize) -> String {
    let mut text = format!("{OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n");
    for _ in 0..calls {
        text.push_str("    attempt(n)\n");
    }
    text.push_str("    return 0\n");
    text
}

fn check(calls: usize) -> Result<(), typecheck::SourceCheckFailure> {
    let files = [parser::parse_strict_file(&source(calls), 0).unwrap()];
    let limits = Limits { work_max: u32::MAX / 2, ..Limits::default() };
    check_module_files_diagnostic("case.fixture", &files, &[], limits).map(|_| ())
}

#[test]
fn outcome_combinations_under_the_cap_check() {
    check(10).unwrap();
}

#[test]
fn outcome_combinations_beyond_the_cap_are_refused_with_the_function_named() {
    let failure = check(12).unwrap_err();
    assert_eq!(failure.error, SourceCheckError::WorldLimit);
    let detail = failure.detail.expect("the refusal explains itself");
    assert!(detail.contains("`run`"), "{detail}");
    assert!(detail.contains("more than 2048 outcome combinations"), "{detail}");
    assert!(failure.culprit.is_some());
}
