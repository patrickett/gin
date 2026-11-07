//! The command line, run as a process.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const UNFORMATTED: &str = "run:\n  return 0   -- done\n";
const FORMATTED: &str = "run:\n    return 0 -- done\n";
const FLAWED: &str = "run:\n    return (\n";

fn ginfmt(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ginfmt")).args(args).output().unwrap()
}

fn ginfmt_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ginfmt")).current_dir(dir).args(args).output().unwrap()
}

fn with_stdin(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginfmt"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn write(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn formats_files_in_place_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "a.gin", UNFORMATTED);
    let output = ginfmt(&[file.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(&output.stdout), "");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), FORMATTED);
    // A second run changes nothing, which --check confirms.
    assert_eq!(ginfmt(&["--check", file.to_str().unwrap()]).status.code(), Some(0));
}

#[test]
fn check_lists_files_that_would_change_writes_nothing_and_exits_one() {
    let dir = tempfile::tempdir().unwrap();
    let dirty = write(dir.path(), "dirty.gin", UNFORMATTED);
    let clean = write(dir.path(), "clean.gin", FORMATTED);
    let output = ginfmt(&["--check", dirty.to_str().unwrap(), clean.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), format!("{}\n", dirty.display()));
    assert_eq!(std::fs::read_to_string(&dirty).unwrap(), UNFORMATTED);
}

#[test]
fn check_of_formatted_files_is_silent_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "clean.gin", FORMATTED);
    let output = ginfmt(&["--check", dir.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(&output.stdout), "");
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn directories_are_searched_for_gin_files_only_and_skip_hidden_and_target() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "sub/a.gin", UNFORMATTED);
    let b = write(dir.path(), "b.gin", UNFORMATTED);
    let note = write(dir.path(), "notes.txt", UNFORMATTED);
    let hidden = write(dir.path(), ".cache/h.gin", UNFORMATTED);
    let built = write(dir.path(), "target/t.gin", UNFORMATTED);
    let output = ginfmt(&["--check", dir.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let listed: Vec<String> = text(&output.stdout).lines().map(str::to_owned).collect();
    assert_eq!(listed, vec![b.display().to_string(), a.display().to_string()]);
    for untouched in [note, hidden, built] {
        assert_eq!(std::fs::read_to_string(untouched).unwrap(), UNFORMATTED);
    }
}

#[test]
fn a_flaw_is_printed_the_file_is_left_alone_and_the_rest_is_still_formatted() {
    let dir = tempfile::tempdir().unwrap();
    let bad = write(dir.path(), "bad.gin", FLAWED);
    let good = write(dir.path(), "good.gin", UNFORMATTED);
    let output = ginfmt_in(dir.path(), &["bad.gin", "good.gin"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("bad.gin"), "{stderr}");
    assert!(stderr.contains("syntax flaw"), "{stderr}");
    assert_eq!(std::fs::read_to_string(&bad).unwrap(), FLAWED);
    assert_eq!(std::fs::read_to_string(&good).unwrap(), FORMATTED);
}

#[test]
fn a_flaw_in_check_mode_is_an_error_not_a_listing() {
    let dir = tempfile::tempdir().unwrap();
    let bad = write(dir.path(), "bad.gin", FLAWED);
    let output = ginfmt(&["--check", bad.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn stdin_is_formatted_to_stdout() {
    let output = with_stdin(&["--stdin"], UNFORMATTED);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(&output.stdout), FORMATTED);
}

#[test]
fn stdin_check_names_stdin_when_it_would_change() {
    let output = with_stdin(&["--stdin", "--check"], UNFORMATTED);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "<stdin>\n");
    let output = with_stdin(&["--stdin", "--check"], FORMATTED);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn stdin_flaw_writes_nothing_to_stdout_and_exits_two() {
    let output = with_stdin(&["--stdin"], FLAWED);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stdout), "");
    assert!(text(&output.stderr).contains("<stdin>"));
}

#[test]
fn usage_errors_exit_two() {
    for args in [&[][..], &["--bogus"], &["--stdin", "a.gin"], &["/no/such/file.gin"]] {
        let output = ginfmt(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(text(&output.stdout), "", "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn help_exits_zero() {
    let output = ginfmt(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(text(&output.stdout).contains("--check"));
}

#[test]
fn an_error_outranks_a_change_in_the_exit_status() {
    let dir = tempfile::tempdir().unwrap();
    let dirty = write(dir.path(), "dirty.gin", UNFORMATTED);
    let bad = write(dir.path(), "bad.gin", FLAWED);
    let output = ginfmt(&["--check", dirty.to_str().unwrap(), bad.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stdout), format!("{}\n", dirty.display()));
}

#[test]
fn a_file_named_directly_needs_no_gin_extension() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "script", UNFORMATTED);
    assert_eq!(ginfmt(&[file.to_str().unwrap()]).status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), FORMATTED);
}
