//! Template syntax is deliberately unsupported; ordinary declarations with these names are not.
use std::process::Command;

#[test]
fn template_strings_are_rejected_without_emitting_an_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("main.gin");
    std::fs::write(&input, "message(): return \"value \\(42)\"\n").unwrap();
    let artifact = directory.path().join("out.mlir");
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "mlir", "--output"])
        .arg(&artifact)
        .arg(&input)
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("parse-unexpected-token-in-format-string"),
        "{output:?}"
    );
    assert!(!artifact.exists());
}

#[test]
fn template_names_are_ordinary_user_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("main.gin");
    std::fs::write(
        &input,
        "render(value in 0 to 255) in 0 to 255: return value\n\
         release(value in 0 to 255) in 0 to 255: return render(value)\n\
         write(value in 0 to 255) in 0 to 255: return release(value)\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "check"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}
