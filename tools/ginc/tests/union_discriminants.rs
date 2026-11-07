//! Union discriminants on the new stack, built for the host and run natively: a text
//! discriminant read at run time is the active variant's bytes (`modules/log_level_test`), and
//! an integer one is the variant's tag, usable as an exit status.
use std::path::{Path, PathBuf};
use std::process::Command;

/// Build `source` (with `modules/core` as `core`) into an executable.
fn build(name: &str, source: &str) -> Result<(tempfile::TempDir, PathBuf), String> {
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        format!("#settings\nmodule: \"{name}\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\n"),
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), source).unwrap();
    let program = directory.path().join("program");
    let built = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(directory.path())
        .output()
        .unwrap();
    if built.status.success() {
        Ok((directory, program))
    } else {
        Err(String::from_utf8_lossy(&built.stderr).into_owned())
    }
}

fn run(program: &Path) -> (String, Option<i32>) {
    let done = Command::new(program).output().unwrap();
    (
        String::from_utf8_lossy(&done.stdout).into_owned(),
        done.status.code(),
    )
}

#[test]
fn log_level_prints_the_text_of_each_level() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/log_level_test");
    let source = std::fs::read_to_string(root.join("main.gin")).unwrap();
    let cases: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("cases.json")).unwrap()).unwrap();
    let (_directory, program) = build("log_level_test", &source).unwrap_or_else(|e| panic!("{e}"));
    let expected = cases[0]["stdout"].as_str().unwrap();
    assert_eq!(run(&program), (expected.to_owned(), Some(0)));
}

const EXIT: &str = "\
#unrestricted
ExitCode is Success   = 0
         or FirstFlaw = 1
         or SecFlaw   = 7

-- A variant that is only known at run time: the one after `code`.
next(code ExitCode) ExitCode: when code is
    Success then ExitCode.FirstFlaw
    FirstFlaw then ExitCode.SecFlaw
    SecFlaw then ExitCode.Success

status(code ExitCode) in 0 to 7: return code.value
";

#[test]
fn an_integer_value_is_the_exit_status() {
    for (expression, expected) in [
        ("ExitCode.Success", 0),
        ("ExitCode.FirstFlaw", 1),
        ("ExitCode.SecFlaw", 7),
        ("next(ExitCode.Success)", 1),
        ("next(next(ExitCode.Success))", 7),
        ("next(ExitCode.SecFlaw)", 0),
    ] {
        let source =
            format!("{EXIT}\n#entry\nmain in 0 to 255:\n    return status({expression})\n");
        let (_directory, program) =
            build("exit_code", &source).unwrap_or_else(|e| panic!("{expression}: {e}"));
        assert_eq!(
            run(&program),
            (String::new(), Some(expected)),
            "{expression}"
        );
    }
}

#[test]
fn a_union_without_values_has_no_value_to_read() {
    let source = "#unrestricted\nColor is Red or Blue\n\n#entry\nmain in 0 to 255:\n    c: Color.Red\n    return c.value\n";
    assert!(build("no_value", source).is_err());
}
