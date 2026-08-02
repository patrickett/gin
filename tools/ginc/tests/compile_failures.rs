use std::fs;
use std::path::PathBuf;
use std::process::Command;

use flask::FlaskConfig;
use ginc::cli::{Args, Emit, Profile};
use ginc::compile::{CompileFailure, CompileResult, GinCompiler};
use test_fixtures::unique_temp_dir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn ginc_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ginc"))
}

fn write_source(path: &PathBuf, contents: &str) {
    fs::write(path, contents).expect("write source file");
}

#[test]
fn no_input_files_is_fatal() {
    let dir = unique_temp_dir("ginc-no-input-failure");
    fs::create_dir_all(&dir).expect("create empty input dir");

    let exe_path = dir.join("out");
    let mut args = Args {
        input: dir.clone(),
        emit: Emit::Exe,
        output: Some(exe_path.clone()),
        profile: Profile::Debug,
        ..Default::default()
    };

    let result = GinCompiler::compile(&mut args);

    assert!(matches!(
        result,
        CompileResult::Failed(CompileFailure::NoInputFiles { path }) if path == dir
    ));
    assert!(!exe_path.exists());
}

#[test]
fn unreadable_source_is_fatal_and_prevents_partial_output() {
    if cfg!(not(unix)) {
        return;
    }

    let dir = unique_temp_dir("ginc-unreadable-source");
    fs::create_dir_all(&dir).expect("create input dir");

    let first = dir.join("00_unreadable.gin");
    let second = dir.join("99_readable.gin");
    write_source(&first, "main: 1\n");
    write_source(&second, "main: 2\n");

    #[cfg(unix)]
    fs::set_permissions(&first, fs::Permissions::from_mode(0o000))
        .expect("set unreadable permissions");

    let exe_path = dir.join("out");
    let mut args = Args {
        input: dir.clone(),
        emit: Emit::Exe,
        output: Some(exe_path.clone()),
        ..Default::default()
    };

    let result = GinCompiler::compile(&mut args);

    assert!(
        matches!(result, CompileResult::Failed(CompileFailure::UnreadableSource { path, .. }) if path == first)
    );
    assert!(!exe_path.exists());
}

#[test]
fn parse_failure_is_fatal_and_renderable() {
    let dir = unique_temp_dir("ginc-parse-failure");
    fs::create_dir_all(&dir).expect("create input dir");

    let source = dir.join("main.gin");
    write_source(&source, "main:\n");

    let mut args = Args {
        input: source.clone(),
        emit: Emit::Mlir,
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);

    assert!(matches!(
        result,
        CompileResult::Failed(CompileFailure::ParseFailed)
    ));

    let status = Command::new(ginc_binary())
        .arg(source)
        .arg("--emit")
        .arg("mlir")
        .output()
        .expect("run ginc binary");
    let stderr = String::from_utf8_lossy(&status.stderr);

    assert!(!status.status.success());
    assert!(stderr.contains("parse-expected-expression"));
}

#[test]
fn invalid_target_is_not_suppressed() {
    let dir = unique_temp_dir("ginc-invalid-target");
    fs::create_dir_all(&dir).expect("create input dir");

    let source = dir.join("main.gin");
    write_source(&source, "main: 1\n");

    let mut args = Args {
        input: source,
        emit: Emit::Exe,
        target: Some("bad_target".to_string()),
        ..Default::default()
    };

    let result = GinCompiler::compile(&mut args);

    assert!(
        matches!(result, CompileResult::Failed(CompileFailure::InvalidTarget { source: Some(s), .. }) if s == "bad_target")
    );
}

#[test]
fn missing_required_entry_target_still_fails_validation() {
    let config = FlaskConfig::new("missing-target".to_string(), "0.0.0".to_string());
    assert!(
        config.require_entry_triple().is_err(),
        "expected default/absent target to be invalid for an entry package"
    );
}

#[test]
fn successful_compilation_is_success() {
    let source = "main: 1\n";
    let source_path = unique_temp_dir("ginc-success");
    fs::create_dir_all(&source_path).expect("create temp dir");
    let source_file = source_path.join("main.gin");
    write_source(&source_file, source);

    let exe_path = source_path.join("out");
    let mut args = Args {
        input: source_file,
        emit: Emit::Exe,
        output: Some(exe_path.clone()),
        ..Default::default()
    };

    let result = GinCompiler::compile(&mut args);

    assert!(matches!(result, CompileResult::Success { output: Some(_) }));
    assert!(exe_path.exists());
}

#[test]
fn typecheck_flaws_do_not_block_emission() {
    let dir = unique_temp_dir("ginc-typecheck-nonfatal");
    fs::create_dir_all(&dir).expect("create input dir");

    let source = dir.join("main.gin");
    write_source(
        &source,
        "main:\n    x: when 1 is\n        2 then 1\n        1 then 2\n        3 then 3\n    return x\n",
    );

    let obj_path = dir.join("main.o");
    let mut args = Args {
        input: source,
        emit: Emit::Obj,
        output: Some(obj_path.clone()),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);

    assert!(result.has_typecheck_diagnostics());
    assert!(result.is_success());
    assert!(obj_path.exists());
}

#[test]
fn no_input_files_exits_nonzero_via_process() {
    let dir = unique_temp_dir("ginc-no-input-process");
    fs::create_dir_all(&dir).expect("create empty input dir");

    let status = Command::new(ginc_binary())
        .arg(&dir)
        .arg("--emit")
        .arg("obj")
        .output()
        .expect("run ginc binary");

    assert!(!status.status.success());
}
