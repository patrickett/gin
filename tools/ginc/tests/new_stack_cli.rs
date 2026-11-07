//! The command line of the new stack (`--emit check` and the `--emit` modes): reporting
//! options apply to every mode, diagnostics agree across formats, and the streams stay apart.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const VALID: &str = include_str!("../../../crates/typecheck/examples/source-echo.gin");
const INVALID: &str =
    include_str!("../../../crates/typecheck/examples/source-echo-invalid.gin");

fn ginc() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
    command
        .env_remove("GIN_PROFILE_JSON")
        .env_remove("GINC_TIMINGS")
        .env_remove("GINC_FAST_LLVM")
        .env("NO_COLOR", "1");
    command
}

fn file(text: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, text).unwrap();
    (directory, path)
}

fn files(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn json_lines(output: &Output) -> Vec<Value> {
    stdout(output)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect()
}

#[test]
fn emit_check_checks_without_producing_anything() {
    let (directory, path) = file(VALID);
    let output = ginc()
        .args(["--emit", "check"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        stderr(&output).contains("Check passed: 0 errors"),
        "{}",
        stderr(&output)
    );
    assert!(stdout(&output).is_empty());
    assert_eq!(files(directory.path()), ["input.gin"]);

    let (_directory, path) = file(INVALID);
    let output = ginc()
        .args(["--emit", "check"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("Check failed: 1 errors"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn emit_check_refuses_options_that_only_a_build_uses() {
    let (directory, path) = file(VALID);
    for option in [
        &["--output", "out"][..],
        &["--profile", "release"],
        &["--fast-llvm"],
        &["--no-cache"],
    ] {
        let output = ginc()
            .args(["--emit", "check"])
            .arg(&path)
            .args(option)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{option:?}: {output:?}");
        assert!(
            stderr(&output).contains("is not supported"),
            "{}",
            stderr(&output)
        );
        assert_eq!(files(directory.path()), ["input.gin"]);
    }
}

/// (code, line, column) of the one diagnostic, read back from each format.
fn located(format: &str, output: &Output) -> (String, u64, u64) {
    match format {
        "json" => {
            let messages = json_lines(output);
            let diagnostic = messages
                .iter()
                .find(|m| m["reason"] == "diagnostic")
                .unwrap();
            (
                diagnostic["code"].as_str().unwrap().to_owned(),
                diagnostic["range"]["start"]["line"].as_u64().unwrap(),
                diagnostic["range"]["start"]["column"].as_u64().unwrap(),
            )
        }
        "short" => {
            // path:line:column: severity[code]: message
            let text = stderr(output);
            let line = text.lines().find(|line| line.contains("flaw[")).unwrap();
            let code = line
                .split("flaw[")
                .nth(1)
                .unwrap()
                .split(']')
                .next()
                .unwrap();
            let mut parts = line.split(':');
            let _path = parts.next().unwrap();
            let row: u64 = parts.next().unwrap().parse().unwrap();
            let column: u64 = parts.next().unwrap().parse().unwrap();
            (code.to_owned(), row, column)
        }
        "human" => {
            // flaw: [code] ...  then  ╭─[ path:line:column ]
            let text = stderr(output);
            let code = text
                .split("flaw: [")
                .nth(1)
                .unwrap()
                .split(']')
                .next()
                .unwrap();
            let place = text
                .split("╭─[ ")
                .nth(1)
                .unwrap()
                .split(" ]")
                .next()
                .unwrap();
            let mut parts = place.rsplit(':');
            let column: u64 = parts.next().unwrap().parse().unwrap();
            let row: u64 = parts.next().unwrap().parse().unwrap();
            (code.to_owned(), row, column)
        }
        other => panic!("{other}"),
    }
}

const NEGATIVES: &[(&str, &str, &str)] = &[
    ("proof obligation", INVALID, "source-check-unproved"),
    ("unsupported", "main: 1 = 2\n", "source-check-unsupported"),
    (
        "unknown name",
        "f(x in 0 to 3) in 0 to 3: return y\n",
        "source-check-unknown-name",
    ),
];

/// Gate G-diag: one diagnostic code and one start position, whatever the format.
#[test]
fn the_same_code_and_range_come_out_of_human_short_and_json() {
    for (what, source, code) in NEGATIVES {
        let (_directory, path) = file(source);
        let mut seen = Vec::new();
        for format in ["human", "short", "json"] {
            let output = ginc()
                .args(["--emit", "check", "--message-format", format])
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(1), "{what} {format}: {output:?}");
            seen.push((format, located(format, &output)));
        }
        for (format, found) in &seen {
            assert_eq!(found.0, *code, "{what} {format}");
            assert_eq!(found, &seen[2].1, "{what}: {format} differs from json");
        }
    }
}

#[test]
fn the_same_codes_come_out_of_a_package_in_every_format() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), INVALID).unwrap();
    let mut seen = Vec::new();
    for format in ["human", "short", "json"] {
        let output = ginc()
            .args(["--emit", "check", "--message-format", format])
            .arg(directory.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{format}: {output:?}");
        seen.push(located(format, &output));
    }
    assert_eq!(seen[0], seen[2]);
    assert_eq!(seen[1], seen[2]);
    assert_eq!(seen[2].0, "source-check-unproved");
}

#[test]
fn reporting_options_apply_to_every_new_stack_mode() {
    let (_directory, path) = file(INVALID);
    for mode in [&["--emit", "check"][..], &["--emit", "mlir"]] {
        let output = ginc()
            .args(mode)
            .arg(&path)
            .args(["--message-format=json", "--max-errors=0"])
            .arg(if mode[1] == "mlir" {
                "--output=/dev/null"
            } else {
                "--quiet"
            })
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{mode:?}: {output:?}");
        let messages = json_lines(&output);
        assert!(
            messages.iter().all(|m| m["reason"] != "diagnostic"),
            "{mode:?}"
        );
        let summary = messages
            .iter()
            .find(|m| m["reason"] == "check-finished")
            .unwrap();
        assert_eq!(summary["omitted_errors"], 1, "{mode:?}");
    }
}

#[test]
fn max_errors_applies_to_a_built_package() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), INVALID).unwrap();
    let output = ginc()
        .args(["--emit", "exe", "--max-errors=0", "--message-format=short"])
        .arg(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("(1 errors omitted)"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn default_path_accepts_reporting_options() {
    let (_directory, path) = file(VALID);
    for option in ["--message-format=json", "--max-errors=1", "--warnings=deps"] {
        let output = ginc()
            .arg(&path)
            .arg(option)
            .arg("--quiet")
            .output()
            .unwrap();
        assert!(output.status.success(), "{option}: {output:?}");
        assert!(
            !stderr(&output).contains("need the new stack"),
            "{}",
            stderr(&output)
        );
    }
}

#[test]
fn removed_stack_selection_aliases_are_rejected() {
    let (_directory, path) = file(VALID);
    for options in [
        &["--check-new"][..],
        &["--emit-new", "mlir"],
        &["--legacyfallback"],
    ] {
        let output = ginc().arg(&path).args(options).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{options:?}: {output:?}");
        assert!(
            stderr(&output).contains("unexpected argument"),
            "{output:?}"
        );
        assert!(output.stdout.is_empty(), "{output:?}");
    }
}

#[test]
fn failure_text_without_a_diagnostic_is_prefixed_error() {
    let (directory, _path) = file(VALID);
    // A directory with no package around it: no diagnostic, only the failure text.
    let output = ginc()
        .args(["--emit", "check"])
        .arg(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let text = stderr(&output);
    assert!(text.contains("error: package failed"), "{text}");
    assert!(!text.contains("begin check"), "{text}");

    let output = ginc()
        .args(["--emit", "check", "--message-format=json"])
        .arg(directory.path())
        .output()
        .unwrap();
    let messages = json_lines(&output);
    assert!(
        messages.iter().any(|m| m["code"] == "check-failed"),
        "{messages:?}"
    );
}

#[test]
fn json_diagnostics_and_mlir_never_share_stdout() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), VALID).unwrap();
    let package = directory.path();
    // Without --output, JSON and MLIR would both want stdout: refused before anything runs.
    let output = ginc()
        .args(["--emit", "mlir", "--message-format=json"])
        .arg(package)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).is_empty() || !stdout(&output).contains("```"));
    assert!(!stdout(&output).contains("func.func"));
    for message in json_lines(&output) {
        assert!(message["reason"] == "diagnostic" || message["reason"] == "check-finished");
    }
    // With --output the MLIR is a file and stdout holds only JSON lines.
    let mlir = package.join("out.mlir");
    let output = ginc()
        .args(["--emit", "mlir", "--message-format=json"])
        .arg("--output")
        .arg(&mlir)
        .arg(package)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let messages = json_lines(&output);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0]["reason"], "check-finished");
    let text = std::fs::read_to_string(&mlir).unwrap();
    assert!(text.contains("app.echo"), "{text}");
    assert!(!text.contains("```"));
    // Human format keeps MLIR on stdout, fenced, as before.
    let output = ginc()
        .args(["--emit", "mlir"])
        .arg(package)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(stdout(&output).contains("```mlir"), "{}", stdout(&output));
}

#[test]
fn no_debug_line_is_left_in_the_lowering() {
    assert!(!include_str!("../src/lower_next.rs").contains("DBGLOWER"));
    let (_directory, path) = file(VALID);
    let output = ginc()
        .args(["--emit", "mlir"])
        .arg(&path)
        .env("DBGLOWER", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!stderr(&output).contains("DBG "), "{}", stderr(&output));
}
