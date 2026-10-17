use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use test_fixtures::TempPackage;

fn run(directory: &Path, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .current_dir(directory)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("interpreter process");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

#[test]
fn no_arguments_start_without_a_manifest_and_pipes_have_no_prompts() {
    let directory = TempPackage::new("interpreter_cli_standalone");
    directory.write("unrelated.gin", "broken := @");
    let output = run(directory.path(), &[], "40 + 2\n#type 42\n");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "42\ninteger in 42 to 42\n");
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
}

#[test]
fn eval_submissions_share_declarations_and_accept_negative_literals() {
    let directory = TempPackage::new("interpreter_cli_eval");
    let output = run(
        directory.path(),
        &["--eval", "answer := 42", "--eval", "answer", "--eval", "-1"],
        "",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "42\n-1\n");
}

#[test]
fn eval_accepts_complete_multiline_code_without_a_blank_line() {
    let directory = TempPackage::new("interpreter_cli_multiline_eval");
    let output = run(
        directory.path(),
        &["--eval", "answer:\n    return 40 + 2", "--eval", "answer"],
        "",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "42\n");
}

#[test]
fn batch_redefinition_preserves_siblings_and_calls_and_queries_need_no_wrapping() {
    let directory = TempPackage::new("interpreter_cli_redefinition");
    let output = run(
        directory.path(),
        &[
            "--eval",
            "answer: 40\nother: 2",
            "--eval",
            "answer: 42",
            "--eval",
            "answer()",
            "--eval",
            "other()",
            "--eval",
            "#size(@())",
        ],
        "",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!("42\n2\n{}\n", std::mem::size_of::<usize>())
    );
}

#[test]
fn batch_failures_allow_feedback_and_repair_but_return_nonzero() {
    let directory = TempPackage::new("interpreter_cli_failure");
    for (args, input) in [
        (
            vec![
                "--eval",
                "missing + 2",
                "--eval",
                "missing := 40",
                "--eval",
                "#retry",
            ],
            "",
        ),
        (vec![], "missing + 2\nmissing := 40\n#retry\n"),
    ] {
        let output = run(directory.path(), &args, input);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("type-unknown-symbol"),
            "{}",
            stderr(&output)
        );
        assert!(
            stdout(&output).lines().any(|line| line == "42"),
            "{}",
            stdout(&output)
        );
    }
}

#[test]
fn end_of_input_submits_complete_multiline_code_and_reports_incomplete_code() {
    let directory = TempPackage::new("interpreter_cli_eof");
    let output = run(directory.path(), &[], "(40 +\n2)");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "42\n");

    for (args, input) in [(vec![], "(40 +\n"), (vec!["--eval", "(40 +"], "")] {
        let output = run(directory.path(), &args, input);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("incomplete submission"),
            "{}",
            stderr(&output)
        );
    }
}

#[test]
fn package_discovery_and_explicit_paths_preserve_imports() {
    let package = TempPackage::new("interpreter_cli_discovery");
    package.write_flask("interpreter_cli_discovery");
    package.write("numbers/defs.gin", "answer := 42\n");
    let nested = package.join("numbers");
    let output = run(&nested, &[], "use 'numbers' answer\nanswer\n");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "42\n");

    let elsewhere = TempPackage::new("interpreter_cli_explicit");
    let output = run(
        elsewhere.path(),
        &[
            package.path().to_str().unwrap(),
            "--eval",
            "use 'numbers' answer",
            "--eval",
            "answer",
        ],
        "",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "42\n");
}

#[test]
fn invalid_manifests_are_reported_and_can_be_explicitly_ignored() {
    let directory = TempPackage::new("interpreter_cli_bad_manifest");
    directory.write("mod.flask", "invalid");
    let output = run(directory.path(), &["--eval", "42"], "");
    assert!(!output.status.success());
    assert!(stderr(&output).contains("invalid package configuration"));

    let output = run(directory.path(), &["--isolated", "--eval", "42"], "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "42\n");

    let output = run(directory.path(), &["--help"], "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("[PACKAGE]"));
    assert!(stdout(&output).contains("--eval"));
    assert!(!stdout(&output).contains("Gin Interpreter"));
}

#[test]
fn scratch_imports_explain_how_to_load_a_package() {
    let directory = TempPackage::new("interpreter_cli_scratch_import");
    let output = run(
        directory.path(),
        &["--eval", "use core", "--eval", "40 + 2"],
        "",
    );
    assert!(!output.status.success());
    assert!(stderr(&output).contains("interpreter-import-requires-package"));
    assert!(stderr(&output).contains("pass its path to ginterpreter"));
    assert_eq!(stdout(&output), "42\n");
}

#[test]
fn help_queries_and_documentation_are_available_to_shell_agents() {
    let directory = TempPackage::new("interpreter_cli_queries");
    let output = run(
        directory.path(),
        &[
            "--eval",
            "#help",
            "--eval",
            "#queries",
            "--eval",
            "answer := 42",
            "--eval",
            "#doc answer",
        ],
        "",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    for expected in [
        "#type <expression>",
        "#scope",
        "#doc <symbol>",
        "#size",
        "#alignment",
        "#index_bits",
        "#target",
        "answer",
    ] {
        assert!(
            stdout(&output).contains(expected),
            "{expected}: {}",
            stdout(&output)
        );
    }
}

#[test]
fn batch_sessions_wait_for_background_evaluation_and_report_its_failure() {
    let directory = TempPackage::new("interpreter_cli_tasks");
    for (args, input, succeeds) in [
        (vec!["--eval", "#run 40 + 2"], "", true),
        (vec![], "#run 40 + 2\n", true),
        (vec!["--eval", "#run missing"], "", false),
    ] {
        let output = run(directory.path(), &args, input);
        assert_eq!(output.status.success(), succeeds, "{}", stderr(&output));
        if succeeds {
            assert!(
                stdout(&output).contains("task 1 => 42"),
                "{}",
                stdout(&output)
            );
        } else {
            assert!(
                stderr(&output).contains("type-unknown-symbol"),
                "{}",
                stderr(&output)
            );
        }
    }
}
