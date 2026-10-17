use std::process::Command;

use ginc::cli::{Args, Emit};
use ginc::compile::{CompileFailure, CompileResult, GinCompiler};
use test_fixtures::{TempPackage, host_target_triple};

const BYTE: &str = "Byte is in 0 to 255\nByte has finish(self): return\n";

#[test]
fn unsafe_integer_programs_do_not_emit_artifacts() {
    let cases = [
        "narrow(value in 0 to 65535) Byte: return value as Byte\n#entry\nmain: return 0\n",
        "divide(value in 0 to 255, divisor in 0 to 10): return value / divisor\n\
         #entry\nmain: return 0\n",
        "shift(value in 0 to 255, count in -1 to 8): return value << count\n\
         #entry\nmain: return 0\n",
        "bounded(value in 0 to 255 and value < 10): return\n\
         #entry\nmain:\n    bounded(10)\n    return 0\n",
        "clear(value mut is in 0 to 255): return thus value = 0\n\
         #entry\nmain: return 0\n",
        "invalid Byte: 256\n#entry\nmain: return 0\n",
        "invalid Byte: -1\n#entry\nmain: return 0\n",
        "accept(value in 0 to 255): return value\n\
         narrow(value in 0 to 65535): return accept(value)\n#entry\nmain: return 0\n",
        "bad(value in 0 to 255): return #emit(0, value)\n#entry\nmain: return 0\n",
    ];
    for (index, source) in cases.into_iter().enumerate() {
        let package = TempPackage::new("integer_safety");
        let input = package.write("main.gin", &format!("{BYTE}{source}"));
        for emit in [Emit::Obj, Emit::Exe] {
            let output = package.join(format!("unsafe-{emit:?}"));
            let result = GinCompiler::compile(&mut Args {
                input: input.clone(),
                output: Some(output.clone()),
                emit,
                target: Some(host_target_triple()),
                no_cache: true,
                ..Default::default()
            });
            assert!(
                matches!(
                    result,
                    CompileResult::Failed(CompileFailure::TypecheckFailed)
                ),
                "case {index}, {emit:?}: {result:?}"
            );
            assert!(!output.exists(), "case {index}: {}", output.display());
        }
    }
}

#[test]
fn unsafe_narrowing_is_a_failing_cli_exit() {
    let package = TempPackage::new("integer_safety_cli");
    let input = package.write(
        "main.gin",
        &format!(
            "{BYTE}narrow(value in 0 to 65535) Byte: return value as Byte\n\
             #entry\nmain: return 0\n"
        ),
    );
    let artifact = package.join("unsafe.o");
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg(input)
        .args([
            "--emit",
            "obj",
            "--target",
            &host_target_triple(),
            "--no-cache",
            "-o",
        ])
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!artifact.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("type-integer-narrowing-failed"),
        "{output:?}"
    );
}
