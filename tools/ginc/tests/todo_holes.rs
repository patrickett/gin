//! A `#todo` hole checks, but lowering refuses it with a clear error instead of compiling it.
use std::process::Command;

fn run(flag: &str, text: &str) -> std::process::Output {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, text).unwrap();
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg(flag)
        .arg("mlir")
        .arg(&path)
        .output()
        .unwrap()
}

const SOURCE: &str = "f(a in 0 to 9) in 0 to 9:\n    when a < 5 then return 1\n    return #todo\n";

#[test]
fn checking_a_hole_succeeds_with_a_warning() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, SOURCE).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "check"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("source-check-hole"), "{text}");
}

#[test]
fn lowering_a_hole_is_refused_and_emits_nothing() {
    let output = run("--emit", SOURCE);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("`#todo` hole"), "{text}");
    assert!(!text.contains("func.func"), "{text}");
}
