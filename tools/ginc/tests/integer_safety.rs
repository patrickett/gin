//! Unsafe integer operations fail before writing native artifacts.
use ginc::cli::{Args, Emit};
use ginc::compile::{CompileFailure, CompileResult, GinCompiler};
use std::process::Command;

#[test]
fn unsafe_integer_programs_do_not_emit_artifacts() {
    for source in [
        "narrow(value in 0 to 65535) in 0 to 255: return value\n",
        "divide(value in 0 to 255, divisor in 0 to 10): return value / divisor\n",
        "shift(value in 0 to 255, count in -1 to 8): return value << count\n",
        "invalid() in 0 to 255: return 256\n",
        "invalid() in 0 to 255: return -1\n",
    ] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("mod.flask"),
            "#settings\nmodule: 'safety'\nversion: 1.0.0\n",
        )
        .unwrap();
        let input = directory.path().join("main.gin");
        std::fs::write(&input, source).unwrap();
        for emit in [Emit::Obj, Emit::Exe] {
            let artifact = directory.path().join(format!("unsafe-{emit:?}"));
            let result = GinCompiler::compile(&mut Args {
                input: input.clone(),
                output: Some(artifact.clone()),
                emit,
                target: Some(test_fixtures::host_target_triple()),
                ..Default::default()
            });
            assert!(
                matches!(
                    result,
                    CompileResult::Failed(CompileFailure::TypecheckFailed)
                ),
                "{result:?}"
            );
            assert!(!artifact.exists(), "{}", artifact.display());
        }
    }
}

#[test]
fn unsafe_integer_result_is_a_failing_cli_exit() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: 'safety'\nversion: 1.0.0\n",
    )
    .unwrap();
    let input = directory.path().join("main.gin");
    std::fs::write(&input, "invalid() in 0 to 255: return 256\n").unwrap();
    let artifact = directory.path().join("unsafe.o");
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "obj", "--message-format=json", "--output"])
        .arg(&artifact)
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!artifact.exists());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("source-check-type"),
        "{output:?}"
    );
}
