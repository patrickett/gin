use std::fs;
use std::process::Command;

use tempfile::tempdir;

fn interpreter() -> Command {
    Command::new(env!("CARGO_BIN_EXE_interpreter"))
}

fn assert_output(args: &[&str], code: i32, stdout: &str, stderr: &str) {
    let output = interpreter().args(args).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), stdout);
    assert_eq!(String::from_utf8(output.stderr).unwrap(), stderr);
}

fn assert_failure(args: &[&str], stderr_prefix: &str) {
    let output = interpreter().args(args).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with(stderr_prefix)
    );
}

#[test]
fn run_file_and_call_function() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("main.gin");
    fs::write(
        &file,
        "#entry\nstart() in 0 to 255:\n    return 7\n\nadd(x in 0 to 10, y in 0 to 10) in 0 to 20:\n    return x + y\n",
    )
    .unwrap();

    let path = file.to_str().unwrap();
    assert_output(&["run", path], 7, "7\n", "");
    assert_output(&["call", path, "add", "2", "3"], 0, "5\n", "");
}

#[test]
fn run_package_directory() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("main.gin"),
        "#entry\nstart() in 0 to 255:\n    return 7\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("mod.flask"),
        "#settings\nmodule: \"cli_test\"\nversion: 0.1.0\n",
    )
    .unwrap();

    assert_output(&["run", dir.path().to_str().unwrap()], 7, "7\n", "");
}

#[test]
fn invalid_invocations_keep_the_cli_status_and_diagnostics() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("main.gin");
    fs::write(&file, "this is not valid Gin\n").unwrap();
    let path = file.to_str().unwrap();

    assert_output(
        &[],
        2,
        "",
        "usage: interpreter run <path> | call <path> <function> [integers...]\n",
    );
    assert_failure(&["run", path], "error: ");

    fs::write(&file, "#entry\nstart() in 0 to 255:\n    return 1\n").unwrap();
    assert_output(&["call", path], 2, "", "call needs a function name\n");
    assert_failure(&["call", path, "start", "nope"], "bad integer argument: ");
    assert_output(&["wat", path], 2, "", "unknown command `wat`\n");
}
