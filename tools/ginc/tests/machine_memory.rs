#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ginc::cli::{Args, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::{TempPackage, host_target_triple};
use typecheck::assembly::{AssemblyInput, AssemblyMemory, AssemblySpec};

fn protocol(
    target: &str,
    inputs: &[(&str, &str)],
    clobbers: &[&str],
    instruction: &str,
) -> AssemblySpec {
    AssemblySpec {
        target: target.into(),
        dialect: if target.starts_with("x86_64") {
            "intel"
        } else {
            "att"
        }
        .into(),
        inputs: inputs
            .iter()
            .map(|(parameter, register)| AssemblyInput {
                parameter: (*parameter).into(),
                register: (*register).into(),
                tie: false,
            })
            .collect(),
        result: Some(
            if target.starts_with("x86_64") {
                "rax"
            } else {
                "x0"
            }
            .into(),
        ),
        outputs: Vec::new(),
        clobbers: clobbers.iter().map(|name| (*name).into()).collect(),
        memory: AssemblyMemory::default(),
        side_effects: true,
        template: instruction.into(),
    }
}

fn source(target: &str) -> Option<String> {
    let (reserve, mut release, mut write) = match target {
        "aarch64-apple-darwin" => (
            protocol(
                target,
                &[("length", "x9")],
                &["x1", "x2", "x3", "x4", "x5", "x16", "cc"],
                "mov x0, #0\nmov x1, x9\nmov x2, #3\nmov x3, #4098\nmov x4, #-1\nmov x5, #0\nmov x16, #197\nsvc #128\nb.cc 1f\nmov x0, #0\n1:",
            ),
            protocol(
                target,
                &[("pointer", "x9"), ("length", "x10")],
                &["x1", "x16", "cc"],
                "mov x0, x9\nmov x1, x10\nmov x16, #73\nsvc #128\ncneg x0, x0, cs",
            ),
            protocol(
                target,
                &[("pointer", "x9"), ("length", "x10")],
                &["x1", "x2", "x16", "cc"],
                "mov x0, #1\nmov x1, x9\nmov x2, x10\nmov x16, #4\nsvc #128\ncneg x0, x0, cs",
            ),
        ),
        "x86_64-unknown-linux-gnu" => (
            protocol(
                target,
                &[("length", "rsi")],
                &["rdi", "rdx", "r10", "r8", "r9", "rcx", "r11", "cc"],
                "mov rax, 9\nxor edi, edi\nmov edx, 3\nmov r10d, 34\nmov r8, -1\nxor r9d, r9d\nsyscall\ncmp rax, -4095\njb 1f\nxor eax, eax\n1:",
            ),
            protocol(
                target,
                &[("pointer", "rdi"), ("length", "rsi")],
                &["rcx", "r11", "cc"],
                "mov rax, 11\nsyscall",
            ),
            protocol(
                target,
                &[("pointer", "rsi"), ("length", "rdx")],
                &["rdi", "rcx", "r11", "cc"],
                "mov rax, 1\nmov edi, 1\nsyscall",
            ),
        ),
        "aarch64-unknown-linux-gnu" => (
            protocol(
                target,
                &[("length", "x9")],
                &["x1", "x2", "x3", "x4", "x5", "x8", "cc"],
                "mov x0, #0\nmov x1, x9\nmov x2, #3\nmov x3, #34\nmov x4, #-1\nmov x5, #0\nmov x8, #222\nsvc #0\ncmn x0, #4095\nb.cc 1f\nmov x0, #0\n1:",
            ),
            protocol(
                target,
                &[("pointer", "x9"), ("length", "x10")],
                &["x1", "x8", "cc"],
                "mov x0, x9\nmov x1, x10\nmov x8, #215\nsvc #0",
            ),
            protocol(
                target,
                &[("pointer", "x9"), ("length", "x10")],
                &["x1", "x2", "x8", "cc"],
                "mov x0, #1\nmov x1, x9\nmov x2, x10\nmov x8, #64\nsvc #0",
            ),
        ),
        _ => return None,
    };
    release
        .memory
        .writes
        .push(("pointer".into(), "length".into()));
    write.memory.reads.push(("pointer".into(), "length".into()));
    let literal = |mut spec: AssemblySpec| {
        spec.template = spec.template.replace('\n', "\\n");
        format!(
            "'{}'",
            spec.canonical_form()
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
        )
    };
    Some(format!(
        r#"Byte is in 0 to 255
Word is in 0 to 18446744073709551615
Status is in -9223372036854775808 to 9223372036854775807

#machine(architecture)
reserve(length Word) @Byte: #asm({}, length: length)
#machine(architecture)
release(pointer @Byte, length Word) Status: #asm({}, pointer: pointer, length: length)
#machine(architecture)
write(pointer @Byte, length Word) Status: #asm({}, pointer: pointer, length: length)

copy_pointer(pointer ref @Byte) @Byte: deref pointer
replace_byte(previous Byte, next Byte) Byte: next

copy(destination mut @Byte, input ref @Byte, length is in 0 to 18446744073709551615):
    for index in 0 to length
        destination.(index):: replace_byte(destination.(index), input.(index))
    loop
return

main Status:
    failed: reserve(0)
    failed_address is in 0 to 18446744073709551615: failed as Word
    failure is in -9223372036854775808 to 9223372036854775807: when failed_address = 0 is True then 0 else 1
    input: reserve(5)
    destination: reserve(5)
    release_input: copy_pointer(ref input)
    invalid_release is in -9223372036854775808 to 9223372036854775807: release(release_input, 0)
    release_failure is in -9223372036854775808 to 9223372036854775807: when invalid_release < 0 is True then 0 else 2
    input.(0):: replace_byte(input.(0), 65 as Byte)
    input.(1):: replace_byte(input.(1), 0 as Byte)
    input.(2):: replace_byte(input.(2), 66 as Byte)
    input.(3):: replace_byte(input.(3), 255 as Byte)
    input.(4):: replace_byte(input.(4), 10 as Byte)
    copy(mut destination, ref input, 5)
    output: copy_pointer(ref destination)
    written is in -9223372036854775808 to 9223372036854775807: write(output, 5)
    output_failure is in -9223372036854775808 to 9223372036854775807: when written = 5 is True then 0 else 4
    released_input is in -9223372036854775808 to 9223372036854775807: release(input, 5)
    released_destination is in -9223372036854775808 to 9223372036854775807: release(destination, 5)
return (failure + release_failure + output_failure + released_input + released_destination) as Status

{}
"#,
        literal(reserve),
        literal(release),
        literal(write),
        test_fixtures::machine_backend_source(target)
    ))
}

fn llvm_nm() -> PathBuf {
    for prefix in ["/opt/homebrew/opt/llvm/bin", "/usr/local/opt/llvm/bin"] {
        let path = Path::new(prefix).join("llvm-nm");
        if path.exists() {
            return path;
        }
    }
    "llvm-nm".into()
}

#[test]
fn machine_memory_allocates_copies_binary_bytes_and_releases_without_runtime() {
    let target = host_target_triple();
    let Some(source) = source(&target) else {
        return;
    };
    assert!(!source.contains("extern"));
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("machine_memory");
        let executable = package.join("program");
        let mut args = Args {
            input: package.write("main.gin", &source),
            output: Some(executable.clone()),
            target: Some(target.clone()),
            profile,
            no_cache: true,
            gin_home: Some(flask::GinHome::new(package.join("home"))),
            ..Default::default()
        };
        let compilation = GinCompiler::compile(&mut args);
        assert!(compilation.is_success(), "{compilation:?}");
        assert!(!compilation.has_typecheck_diagnostics(), "{compilation:?}");
        let execution = Command::new(&executable).output().unwrap();
        assert!(execution.status.success(), "{execution:?}");
        assert_eq!(execution.stdout, [65, 0, 66, 255, 10]);
        let symbols = Command::new(llvm_nm())
            .arg("--just-symbol-name")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(symbols.status.success(), "{symbols:?}");
        assert!(
            !String::from_utf8_lossy(&symbols.stdout).contains("runtime_"),
            "{symbols:?}"
        );
    }
}
