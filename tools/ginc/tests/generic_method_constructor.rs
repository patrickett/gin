//! End-to-end ginc test: declare and use a parameterized-receiver method.
//!
//! Compiles a small program defining `Range(x).new(start x, end x) Range(x): Self(start, end)`
//! and calling `Range.new(12, 1200)`, asserting the resulting record's
//! `.start` field can be returned from main and observed as the exit code.
//!
//! Multi-file tests use the same capability-focused `Range` definitions inline.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use common::*;

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::host_target_triple;

fn ginc_debug_exe() -> PathBuf {
    let exe = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/ginc");
    assert!(
        exe.exists(),
        "ginc binary missing at {}; run `cargo build -p ginc`",
        exe.display()
    );
    exe
}

const RANGE_PROGRAM: &str = "Range(x) has\n    finish(self):\n        return\n    start x\n    end x\n\n    new(start x, end x) Range(x): return Self(start, end)\n\n#entry\nmain:\n    r : Range.new(12, 1200)\n    return r.start\n";

/// Capability-focused `Range` definitions.
const GIN_CORE_RANGE_GIN: &str = "Range(x) has\n    finish(self):\n        return\n    start x\n    end x\n\n    new(start x, end x) Range(x): return Self(start, end)\n";

const RANGE_PKG_MAIN: &str =
    "use Range\n\n#entry\nmain:\n    r : Range.new(12, 1200)\n    return r.start\n";

#[test]
fn range_new_compiles_and_runs() {
    compile_and_run_with_options(
        RANGE_PROGRAM,
        Options {
            test_name: "range_new_constructor".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(12);
}

const STANDALONE_PKG_FLASK: &str = r#"#settings
module: "range_dir_smoke"
version: 0.0.0

#dependencies
"#;

fn unique_temp_pkg(name: &str) -> PathBuf {
    test_fixtures::unique_temp_dir(name)
}

fn write_pkg_file(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Compiles a flask **folder module** (single `main.gin` with capability-focused `Range`).
/// Emits MLIR and checks `Range.new` is present.
///
/// Multi-file `use Range` without `resolve_and_prepare` does not merge sibling files;
/// cross-file package MLIR is covered once directory resolve is wired for dep-less pkgs.
#[test]
fn range_new_pkg_folder_emits_mlir_with_range_fixture() {
    let dir = unique_temp_pkg("range_pkg_mlir");
    let _ = fs::remove_dir_all(&dir);

    fs::create_dir_all(&dir).unwrap();
    write_pkg_file(&dir.join("mod.flask"), STANDALONE_PKG_FLASK);
    let main = format!(
        "{GIN_CORE_RANGE_GIN}\n\n#entry\nmain:\n    r : Range.new(12, 1200)\n    return r.start\n"
    );
    write_pkg_file(&dir.join("main.gin"), &main);

    let target = host_target_triple();
    let out = Command::new(ginc_debug_exe())
        .arg(&dir)
        .args(["--emit", "mlir", "--target", &target])
        .output()
        .expect("spawn ginc for mlir probe");

    assert!(
        out.status.success(),
        "expected ginc mlir to succeed.\nstderr={}\nstdout={}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout),
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("@Range.new") || stdout.contains("Range.new"),
        "expected mlir to mention Range.new.\nstderr={}",
        String::from_utf8_lossy(&out.stderr),
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn range_new_pkg_folder_compiles_exe_with_output_flag() {
    let dir = unique_temp_pkg("range_pkg_exe");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    write_pkg_file(&dir.join("mod.flask"), STANDALONE_PKG_FLASK);
    write_pkg_file(&dir.join("range.gin"), GIN_CORE_RANGE_GIN);
    write_pkg_file(&dir.join("main.gin"), RANGE_PKG_MAIN);
    let exe_path = dir.join("runner_main");
    let mut args = Args {
        input: dir.clone(),
        emit: Emit::Exe,
        output: Some(exe_path.clone()),
        profile: Profile::Debug,
        target: Some(host_target_triple()),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(result.is_success(), "{result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    let output = Command::new(&exe_path)
        .output()
        .expect("run range constructor example");
    assert_eq!(output.status.code(), Some(12), "{output:?}");
    let _ = fs::remove_dir_all(&dir);
}
