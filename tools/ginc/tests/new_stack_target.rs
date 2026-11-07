//! `#target` follows `--target`, else the package's `mod.flask` `target:`, else the host.
use std::process::{Command, Output};

const PICK: &str = "pick() in 0 to 9: return when #target.triple is\n    'aarch64-apple-darwin' then 1\n    'x86_64-apple-darwin' then 2\n    'aarch64-unknown-linux-gnu' then 3\n    'x86_64-unknown-linux-gnu' then 4\n    else 0\n#entry\nstart() in 0 to 9: return pick()\n";

fn package(manifest_target: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let manifest =
        format!("#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"{manifest_target}\"\n");
    std::fs::write(directory.path().join("mod.flask"), manifest).unwrap();
    std::fs::write(directory.path().join("main.gin"), PICK).unwrap();
    directory
}

fn emit(directory: &std::path::Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "mlir"])
        .args(extra)
        .arg(directory)
        .output()
        .unwrap()
}

/// The constant `pick` returns in the emitted MLIR: which arm of the `when` was taken.
fn arm(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    let body = text
        .split("func.func")
        .find(|f| {
            f.trim_start().starts_with("@")
                && f.contains("pick")
                && !f.contains("start")
                && f.contains("arith.constant")
        })
        .expect("pick is emitted");
    let line = body
        .lines()
        .find(|l| l.contains("arith.constant"))
        .expect("a constant");
    line.split("arith.constant ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .to_owned()
}

fn host_arm() -> &'static str {
    if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        "1"
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        "2"
    } else if cfg!(target_arch = "aarch64") {
        "3"
    } else {
        "4"
    }
}

#[test]
fn each_target_takes_its_own_arm() {
    let directory = package("library");
    for (triple, expected) in [
        ("aarch64-apple-darwin", "1"),
        ("x86_64-apple-darwin", "2"),
        ("aarch64-unknown-linux-gnu", "3"),
        ("x86_64-unknown-linux-gnu", "4"),
    ] {
        assert_eq!(
            arm(&emit(directory.path(), &["--target", triple])),
            expected,
            "{triple}"
        );
    }
}

#[test]
fn the_default_is_the_host_and_a_library_manifest_does_not_change_it() {
    let directory = package("library");
    assert_eq!(arm(&emit(directory.path(), &[])), host_arm());
}

#[test]
fn the_manifest_target_is_used_and_the_option_overrides_it() {
    let directory = package("x86_64-unknown-linux-gnu");
    assert_eq!(arm(&emit(directory.path(), &[])), "4");
    // Negative control: the option wins, so the Linux arm is not what a Darwin target gets.
    let darwin = arm(&emit(
        directory.path(),
        &["--target", "aarch64-apple-darwin"],
    ));
    assert_eq!(darwin, "1");
    assert_ne!(darwin, "4");
}

#[test]
fn an_unknown_target_is_refused_and_the_supported_ones_are_listed() {
    let directory = package("library");
    let output = emit(directory.path(), &["--target", "riscv64-unknown-none"]);
    assert!(!output.status.success(), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        text.contains("unknown target `riscv64-unknown-none`"),
        "{text}"
    );
    assert!(
        text.contains("x86_64-unknown-linux-gnu") && text.contains("aarch64-apple-darwin"),
        "{text}"
    );
    let unknown_manifest = package("riscv64-unknown-none");
    assert!(!emit(unknown_manifest.path(), &[]).status.success());
}

#[test]
fn a_single_file_check_honours_the_option_too() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(
        &path,
        PICK.replace("#entry\nstart() in 0 to 9: return pick()\n", ""),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "mlir", "--target", "x86_64-unknown-linux-gnu"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(arm(&output), "4");
}
