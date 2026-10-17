mod common;

use common::compile_exe_checked;
use ginc::cli::{Args, Emit, Profile};
use ginc::compile::{CompileFailure, CompileResult, GinCompiler};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use test_fixtures::{TempPackage, host_target_triple};

const SIGNED_SOURCE: &str = r#"use core.primitive SignedBigInt, IntegerResult

report(result IntegerResult, expected in -9223372036854775808 to 9223372036854775807):
    return when result is
        Flaw(failure) then when failure is
            Overflow then 1
            DivideByZero then 2
            InvalidShift then 3
        else
            value is in SignedBigInt: result
            return when value = expected then 0 else 4

#entry
main:
    operation is in RuntimeInt: runtime_number
    left is in RuntimeInt: runtime_number
    right is in RuntimeInt: runtime_number
    expected is in RuntimeInt: runtime_number
    a: left as SignedBigInt
    b: right as SignedBigInt
    result: when operation is
        0 then a + b
        1 then a - b
        2 then a * b
        3 then a << b
        else a / b
    return report(result, expected)
"#;

const MEMORY_SOURCE: &str = r#"use core.alloc.system MemorySize, MemorySizeResult

report(result MemorySizeResult, expected in 0 to 18446744073709551615):
    return when result is
        Flaw(failure) then when failure is
            Overflow then 1
            Underflow then 2
        else
            value is in MemorySize: result
            return when value = expected then 0 else 4

#entry
main:
    operation is in RuntimeInt: runtime_number
    left is in RuntimeWord: runtime_word_bits(runtime_number)
    right is in RuntimeWord: runtime_word_bits(runtime_number)
    expected is in RuntimeWord: runtime_word_bits(runtime_number)
    a: left as MemorySize
    b: right as MemorySize
    result: when operation = 0 then a + b else a - b
    return report(result, expected)
"#;

fn package(name: &str, source: &str) -> (TempPackage, PathBuf) {
    let package = TempPackage::new(name);
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    package.copy_tree(&core, "core");
    package.write_flask_with_deps(name, "core: './core'\n");
    let input = package.write("main.gin", &test_fixtures::with_runtime(source));
    (package, input)
}

fn run(executable: &Path, profile: Profile, case: (u8, i64, i64, i64, i32)) {
    let (operation, left, right, expected, status) = case;
    let mut process = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        process.stdin.take().unwrap(),
        "{operation} {left} {right} {expected}"
    )
    .unwrap();
    let output = process.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(status),
        "{profile:?}/{case:?}: {output:?}"
    );
    assert!(output.stdout.is_empty(), "{profile:?}/{case:?}: {output:?}");
    assert!(output.stderr.is_empty(), "{profile:?}/{case:?}: {output:?}");
}

#[test]
fn signed_arithmetic_returns_checked_results_at_runtime_boundaries() {
    let (package, input) = package("checked_signed_arithmetic", SIGNED_SOURCE);
    let cases = [
        (0, i64::MAX, 0, i64::MAX, 0),
        (0, i64::MIN, 0, i64::MIN, 0),
        (0, i64::MAX, 1, 0, 1),
        (0, i64::MIN, -1, 0, 1),
        (0, i64::MAX, i64::MIN, -1, 0),
        (1, i64::MIN, 0, i64::MIN, 0),
        (1, i64::MAX, 0, i64::MAX, 0),
        (1, i64::MIN, 1, 0, 1),
        (1, i64::MAX, -1, 0, 1),
        (1, i64::MIN, i64::MIN, 0, 0),
        (2, i64::MIN, 1, i64::MIN, 0),
        (2, i64::MAX, 1, i64::MAX, 0),
        (2, i64::MIN, -1, 0, 1),
        (2, i64::MAX, 2, 0, 1),
        (2, i64::MIN, 0, 0, 0),
        (2, -3, 7, -21, 0),
        (3, -1, 63, i64::MIN, 0),
        (3, 1, 62, 1i64 << 62, 0),
        (3, 1, 63, 0, 1),
        (3, i64::MAX, 1, 0, 1),
        (3, i64::MIN, 0, i64::MIN, 0),
        (3, 0, 63, 0, 0),
        (3, 1, -1, 0, 3),
        (3, 1, 64, 0, 3),
        (3, 0, 255, 0, 3),
        (4, i64::MIN, -1, 0, 1),
        (4, i64::MIN, 1, i64::MIN, 0),
        (4, i64::MAX, -1, -i64::MAX, 0),
        (4, -21, 3, -7, 0),
        (4, 1, 0, 0, 2),
    ];
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("signed-{profile:?}"));
        compile_exe_checked(input.clone(), &executable, profile);
        for case in cases {
            run(&executable, profile, case);
        }
    }
}

#[test]
fn memory_size_arithmetic_returns_overflow_and_underflow() {
    let (package, input) = package("checked_memory_arithmetic", MEMORY_SOURCE);
    let cases = [
        (0, -1, 0, -1, 0),
        (0, -1, 1, 0, 1),
        (0, i64::MAX, 1, i64::MIN, 0),
        (1, 1, 1, 0, 0),
        (1, 0, 1, 0, 2),
        (1, -1, 0, -1, 0),
        (1, -1, -1, 0, 0),
    ];
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("memory-{profile:?}"));
        compile_exe_checked(input.clone(), &executable, profile);
        for case in cases {
            run(&executable, profile, case);
        }
    }
}

#[test]
fn checked_results_cannot_be_used_as_unchecked_integers() {
    for expression in ["left + right", "(left + right) as SignedBigInt"] {
        let source = format!(
            "use core.primitive SignedBigInt\n\
             unchecked(left SignedBigInt, right SignedBigInt) SignedBigInt: return {expression}\n\
             #entry\nmain: return 0\n"
        );
        let (package, input) = package("checked_result_required", &source);
        let artifact = package.join("unsafe.o");
        let result = GinCompiler::compile(&mut Args {
            input,
            output: Some(artifact.clone()),
            emit: Emit::Obj,
            target: Some(host_target_triple()),
            no_cache: true,
            ..Default::default()
        });
        assert!(
            matches!(
                result,
                CompileResult::Failed(CompileFailure::TypecheckFailed)
            ),
            "{expression}: {result:?}"
        );
        assert!(!artifact.exists(), "{expression}: {}", artifact.display());
    }
}
