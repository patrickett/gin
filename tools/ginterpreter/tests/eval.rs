//! `--eval`: each argument is one whole submission against one session, for shell scripts and
//! agents. A flaw in any submission makes the exit status nonzero without stopping the rest.
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .args(args)
        .output()
        .expect("interpreter process")
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

#[test]
fn submissions_share_declarations_and_accept_negative_literals() {
    let output = run(&["--eval", "five() in 0 to 9: return 5", "--eval", "five() + 1", "--eval", "-1"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("ok: added fn five/0"), "{}", stdout(&output));
    assert!(stdout(&output).contains("6 : 6 to 6"), "{}", stdout(&output));
    assert!(stdout(&output).contains("-1 : -1 to -1"), "{}", stdout(&output));
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
}

#[test]
fn multiline_code_is_one_submission_without_a_blank_line() {
    let output = run(&["--eval", "answer() in 0 to 99:\n    return 40 + 2", "--eval", "answer()"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("42 : 42 to 42"), "{}", stdout(&output));
}

#[test]
fn redefinition_replaces_one_declaration_and_keeps_its_siblings() {
    let output = run(&[
        "--eval",
        "first() in 0 to 99: return 40\nsecond() in 0 to 99: return 2",
        "--eval",
        "first() in 0 to 99: return 42",
        "--eval",
        "first()",
        "--eval",
        "second()",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("replaced fn first/0"), "{}", stdout(&output));
    let values: Vec<&str> = stdout(&output).lines().filter(|line| line.contains(" : ")).collect();
    assert_eq!(values, ["42 : 42 to 42", "2 : 2 to 2"], "{}", stdout(&output));
}

#[test]
fn a_flaw_fails_the_run_but_a_later_submission_can_repair_it() {
    let output = run(&[
        "--eval",
        "missing()",
        "--eval",
        "missing() in 0 to 9: return 4",
        "--eval",
        "missing()",
    ]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("source-check-unknown-name"), "{}", stderr(&output));
    assert!(stdout(&output).contains("4 : 4 to 4"), "{}", stdout(&output));
}

#[test]
fn a_rejected_declaration_fails_the_run_and_leaves_the_session_unchanged() {
    let output = run(&["--eval", "bad(n in 0 to 9) in 0 to 9: return n - 3", "--eval", "bad(5)"]);
    assert!(!output.status.success());
    assert!(stdout(&output).contains("rejected"), "{}", stdout(&output));
    assert!(stderr(&output).contains("source-check-type"), "{}", stderr(&output));
}

#[test]
fn incomplete_code_fails_the_run() {
    let output = run(&["--eval", "(40 +"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("parse-"), "{}", stderr(&output));
}

#[test]
fn a_command_is_a_submission_and_quit_ends_the_batch() {
    let output = run(&["--eval", "$scope", "--eval", "$quit", "--eval", "40 + 2"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stdout(&output).contains("42"), "{}", stdout(&output));

    let output = run(&["--eval", "$nonsense"]);
    assert!(!output.status.success());
    // A `$` name that is no command is not Gin either: the lexer reports it.
    assert!(stderr(&output).contains("lex-"), "{}", stderr(&output));
}

#[test]
fn eval_and_jsonl_are_exclusive() {
    let output = run(&["--jsonl", "--eval", "1"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("cannot be used with"), "{}", stderr(&output));
}
