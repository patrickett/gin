use std::fs;
use std::path::PathBuf;
use std::process::Command;

use flask::FlaskConfig;
use ginc::cli::{Args, Emit, Profile};
use ginc::compile::{CompileFailure, CompileResult, GinCompiler};
use test_fixtures::{host_target_triple, unique_temp_dir};

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
        matches!(result, CompileResult::Failed(CompileFailure::InvalidTarget(flask::TargetError::InvalidTriple { value: s, .. })) if s == "bad_target")
    );
}

#[test]
fn parse_failure_precedes_invalid_target() {
    let dir = unique_temp_dir("ginc-parse-before-target");
    fs::create_dir_all(&dir).expect("create input dir");

    let source = dir.join("main.gin");
    write_source(&source, "main:\n");

    let mut args = Args {
        input: source,
        emit: Emit::Exe,
        target: Some("bad_target".to_string()),
        ..Default::default()
    };

    let result = GinCompiler::compile(&mut args);

    assert!(matches!(
        result,
        CompileResult::Failed(CompileFailure::ParseFailed)
    ));
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
        target: Some(host_target_triple()),
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
        target: Some(host_target_triple()),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);

    assert!(result.has_typecheck_diagnostics());
    assert!(result.is_success());
    assert!(obj_path.exists());
}

#[test]
fn interface_emission_succeeds() {
    let dir = unique_temp_dir("ginc-interface-success");
    fs::create_dir_all(&dir).expect("create temp dir");

    let source = dir.join("main.gin");
    write_source(&source, "main:\n    return 1\n");

    let interface_path = dir.join("main.ginif");
    let mut args = Args {
        input: source.clone(),
        emit: Emit::Interface,
        output: Some(interface_path.clone()),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);

    assert!(
        matches!(result, CompileResult::Success { output: Some(path) } if path == interface_path)
    );
    assert!(interface_path.exists());
    let bytes = fs::read(&interface_path).expect("read interface artifact");
    assert!(!bytes.is_empty());
}

#[test]
fn interface_emission_with_warnings_still_succeeds() {
    let dir = unique_temp_dir("ginc-interface-warnings");
    fs::create_dir_all(&dir).expect("create temp dir");

    let source = dir.join("main.gin");
    write_source(
        &source,
        "main:\n    x: when 1 is\n        2 then 1\n        1 then 2\n        3 then 3\n    return x\n",
    );

    let interface_path = dir.join("main.ginif");
    let mut args = Args {
        input: source,
        emit: Emit::Interface,
        output: Some(interface_path.clone()),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);

    assert!(result.has_typecheck_diagnostics());
    assert!(result.is_success());
    assert!(interface_path.exists());
}

#[test]
fn interface_emission_failed_build_does_not_clobber_prior_artifact() {
    let dir = unique_temp_dir("ginc-interface-stale-on-failure");
    fs::create_dir_all(&dir).expect("create temp dir");

    let source = dir.join("main.gin");
    write_source(&source, "main:\n    return 1\n");

    let interface_path = dir.join("main.ginif");
    let mut args = Args {
        input: source.clone(),
        emit: Emit::Interface,
        output: Some(interface_path.clone()),
        ..Default::default()
    };
    let first = GinCompiler::compile(&mut args);
    assert!(
        matches!(first, CompileResult::Success { output: Some(path) } if path == interface_path)
    );
    let stale_bytes = fs::read(&interface_path).expect("read first interface artifact");

    write_source(&source, "main:\n");
    let second = GinCompiler::compile(&mut args);
    assert!(matches!(
        second,
        CompileResult::Failed(CompileFailure::ParseFailed)
    ));

    let current_bytes = fs::read(&interface_path).expect("read interface artifact");
    assert_eq!(
        current_bytes, stale_bytes,
        "failed emission should preserve prior artifact"
    );
}

#[test]
fn interface_emission_is_deterministic_across_file_order() {
    let dir = unique_temp_dir("ginc-interface-file-order");
    fs::create_dir_all(&dir).expect("create temp dir");

    let a = dir.join("a.gin");
    let b = dir.join("b.gin");
    write_source(&a, "main:\n    return 1\n\nx:\n    return 2\n");
    write_source(&b, "y:\n    return 3\n\nz:\n    return 4\n");

    let interface_path = dir.join("main.ginif");
    let mut args = Args {
        input: dir.clone(),
        emit: Emit::Interface,
        output: Some(interface_path.clone()),
        ..Default::default()
    };
    let first = GinCompiler::compile(&mut args);
    assert!(
        matches!(first, CompileResult::Success { output: Some(path) } if path == interface_path)
    );
    let first_bytes = fs::read(&interface_path).expect("read interface artifact");

    write_source(&a, "x:\n    return 2\n\nmain:\n    return 1\n");
    write_source(&b, "z:\n    return 4\n\ny:\n    return 3\n");

    let second = GinCompiler::compile(&mut args);
    assert!(
        matches!(second, CompileResult::Success { output: Some(path) } if path == interface_path)
    );
    let second_bytes = fs::read(&interface_path).expect("read interface artifact");

    assert_eq!(
        first_bytes, second_bytes,
        "interface artifact should be deterministic across file ordering"
    );
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
