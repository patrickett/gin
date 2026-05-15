//! End-to-end ginc test: declare and use a parameterized-receiver method.
//!
//! Compiles a small program defining `Range[x].new(start x, end x) Range[x]: (start, end)`
//! and calling `Range.new(12, 1200)`, asserting the resulting record's
//! `.start` field can be returned from main and observed as the exit code.
//!
//! A second test concatenates the repo’s `modules/gin_core/range.gin` with a
//! small `main`, so any edit to that checked-in file must still compile in this
//! scenario (Phase 5.3 parity: that file plus a caller).

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use common::*;

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;

fn ginc_debug_exe() -> PathBuf {
    let exe = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/ginc");
    assert!(
        exe.exists(),
        "ginc binary missing at {}; run `cargo build -p ginc`",
        exe.display()
    );
    exe
}

const RANGE_PROGRAM: &str = "\
Range[x] has (start x, end x)

Range[x].new(start x, end x) Range[x]: (start, end)

main:
    r := Range.new(12, 1200)
    return r.start
return
";

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

/// On-disk `modules/gin_core/range.gin`, plus a caller, in one translation
/// unit (`Range.new` stays unqualified — no `use core` / `core.Range.*` path).
#[test]
fn range_new_single_file_uses_repo_range_gin_text() {
    let src = format!(
        "{}\n\n{}",
        include_str!("../../../modules/gin_core/range.gin"),
        "main:\n    r := Range.new(12, 1200)\n    return r.start\nreturn\n",
    );
    compile_and_run_with_options(
        &src,
        Options {
            test_name: "range_new_from_repo_range_gin".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(12);
}

const STANDALONE_PKG_FLASK: &str = r#"{
  "name": "range_dir_smoke",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {}
}"#;

fn unique_temp_pkg(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    dir.push(format!("gin_{name}_{pid}_{nanos}"));
    dir
}

fn write_pkg_file(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Compiles a flask **folder module** where `range.gin` matches the repo file
/// and `main.gin` calls `Range.new` (no import — same as a local multi-file
/// package). Emits MLIR and checks `Range.new` is present.
///
/// Executable emission for directory inputs is brittle in `ginc` today; MLIR
/// still exercises parse, resolve, typecheck, and lowering for multi-file pkgs.
#[test]
fn range_new_pkg_folder_emits_mlir_with_repo_range_gin() {
    let repo_range_gin = include_str!("../../../modules/gin_core/range.gin");
    let dir = unique_temp_pkg("range_pkg_mlir");
    let _ = fs::remove_dir_all(&dir);

    fs::create_dir_all(&dir).unwrap();
    write_pkg_file(&dir.join("flask.jsonc"), STANDALONE_PKG_FLASK);
    write_pkg_file(&dir.join("range.gin"), repo_range_gin);
    write_pkg_file(
        &dir.join("main.gin"),
        "use Range\n\nmain:\n    r := Range.new(12, 1200)\n    return r.start\nreturn\n",
    );

    // MLIR emits via println! inside the compiler; run ginc as a subprocess so we
    // can assert on captured stdout without printing into the test harness output.
    let out = Command::new(ginc_debug_exe())
        .arg(&dir)
        .args(["--emit", "mlir"])
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
    let repo_range_gin = include_str!("../../../modules/gin_core/range.gin");
    let dir = unique_temp_pkg("range_pkg_exe");
    let _ = fs::remove_dir_all(&dir);

    fs::create_dir_all(&dir).unwrap();
    write_pkg_file(&dir.join("flask.jsonc"), STANDALONE_PKG_FLASK);
    write_pkg_file(&dir.join("range.gin"), repo_range_gin);
    write_pkg_file(
        &dir.join("main.gin"),
        "use Range\n\nmain:\n    r := Range.new(12, 1200)\n    return r.start\nreturn\n",
    );

    let exe_path = dir.join("runner_main");
    let mut args = Args {
        input: dir.clone(),
        emit: Emit::Exe,
        output: Some(exe_path.clone()),
        profile: Profile::Debug,
        ..Default::default()
    };
    GinCompiler::compile(&mut args);

    assert!(
        exe_path.exists(),
        "expected exe at {} (dir package + --output corrupts linkage if `.o` reuses exe path)",
        exe_path.display()
    );

    let out = Command::new(&exe_path)
        .output()
        .expect("run compiled folder-module exe");

    assert_eq!(
        out.status.code(),
        Some(12),
        "runner exit {:?}",
        out.status.code()
    );

    let _ = fs::remove_dir_all(&dir);
}
