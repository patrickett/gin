//! Gin reaching libc through one mapped instruction: no `extern`, no signature, no C
//! knowledge in the compiler. The ABI is the Gin below.
#![cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::path::Path;
use std::process::{Command, Output};

/// Everything a C call may destroy on arm64 macOS. x18 is reserved; x19-x28 are preserved.
const TARGET: &str = "\
#unrestricted
#wrap
Word is in 0 to 18446744073709551615

#unrestricted
CallerSaved has
    #register('x0')
    result Word
    #register('x1')
    x1 Word
    #register('x2')
    x2 Word
    #register('x3')
    x3 Word
    #register('x4')
    x4 Word
    #register('x5')
    x5 Word
    #register('x6')
    x6 Word
    #register('x7')
    x7 Word
    #register('x8')
    x8 Word
    #register('x9')
    x9 Word
    #register('x10')
    x10 Word
    #register('x11')
    x11 Word
    #register('x12')
    x12 Word
    #register('x13')
    x13 Word
    #register('x14')
    x14 Word
    #register('x15')
    x15 Word
    #register('x16')
    x16 Word
    #register('x17')
    x17 Word
    #register('lr')
    link Word

#instruction('bl $target')
branch_link(target, input Type, output Type, inputs input) output
";

const IMPORTS: &str = "use app.libc put, magnitude, announce, Mode\n";

const LIBC: &str = "\
use app.arm64 Word, CallerSaved, branch_link

-- The C functions this library reaches, each named once. `to_string` is read by the
-- compiler, so no text reaches the program and a typo is an unknown variant.
#unrestricted
Symbol is Putchar
       or Labs

Symbol has
    to_string(self): when self is
        Putchar then 'putchar'
        Labs then 'labs'

OneArgument has
    #register('x0')
    value Word

-- Which letter to print is chosen at run time, from a value that is only a tag.
#unrestricted
Mode is Loud
     or Quiet
     or Plain

Mode has
    letter(self) in 0 to 255: when self is
        Loud then 76
        Quiet then 81
        Plain then 80

announce(mode Mode) in 0 to 255:
    put(mode.letter)
    return 0

put(character Word) Word:
    after := branch_link(Symbol.Putchar.to_string, OneArgument, CallerSaved, OneArgument(value: character))
    return after.result

magnitude(value Word) Word:
    after := branch_link(Symbol.Labs.to_string, OneArgument, CallerSaved, OneArgument(value: value))
    return after.result
";

/// A package of real modules: the target, the C library on top of it, and the program.
fn run(main: &str) -> (Output, String) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.gin"), format!("{IMPORTS}{main}")).unwrap();
    for (name, text) in [("arm64", TARGET), ("libc", LIBC)] {
        let module = root.join(name);
        std::fs::create_dir(&module).unwrap();
        std::fs::write(module.join(format!("{name}.gin")), text).unwrap();
    }
    std::fs::write(
        root.join(flask::MANIFEST_NAME),
        "#settings\nmodule: 'app'\nversion: 0.0.0\n\n#dependencies\n",
    )
    .unwrap();
    let mlir = ginc(root, &["mlir"]);
    let program = root.join("program");
    ginc(root, &["exe", "--output", program.to_str().unwrap()]);
    (Command::new(&program).output().unwrap(), mlir)
}

fn ginc(root: &Path, emit: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg("--emit")
        .args(emit)
        .arg(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn a_call_is_one_inline_instruction_with_no_clobber_list_and_no_signature() {
    let (_, mlir) = run("#entry\nstart() in 0 to 255:\n    put(72)\n    return 0\n");
    assert!(mlir.contains("llvm.func @putchar()"), "{mlir}");
    assert!(mlir.contains("\"bl ${19}\""), "{mlir}");
    assert!(mlir.contains("={lr}"), "{mlir}");
    assert!(
        !mlir.contains("~{x"),
        "registers are outputs, not clobbers: {mlir}"
    );
    assert!(!mlir.contains("~{lr}"), "{mlir}");
}

#[test]
fn gin_prints_through_libc() {
    let (output, _) =
        run("#entry\nstart() in 0 to 255:\n    put(72)\n    put(105)\n    put(10)\n    return 7\n");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hi\n");
    assert_eq!(output.status.code(), Some(7));
}

/// `put(72)` has a known argument and `say` a constant result: only the effect remains.
#[test]
fn an_effect_with_known_inputs_still_runs_and_runs_in_order() {
    let (output, _) = run("\
say() in 0 to 5:
    put(65)
    return 3

#entry
start() in 0 to 255:
    first := say()
    second := say()
    put(66)
    return first + second
");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "AAB");
    assert_eq!(output.status.code(), Some(6));
}

/// A value computed in Gin arrives in `x0`, and libc's answer comes back from `x0`.
#[test]
fn values_cross_the_boundary_both_ways() {
    let (output, _) = run("\
#entry
start() in 0 to 255:
    put(magnitude(83))
    return 0
");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "S");
}

fn build_with_target(target: &str) -> (tempfile::TempDir, Output) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = "#entry\nstart() in 0 to 255:\n    put(72)\n    return 0\n";
    std::fs::write(root.join("main.gin"), format!("{IMPORTS}{main}")).unwrap();
    for (name, text) in [("arm64", target), ("libc", LIBC)] {
        let module = root.join(name);
        std::fs::create_dir(&module).unwrap();
        std::fs::write(module.join(format!("{name}.gin")), text).unwrap();
    }
    std::fs::write(
        root.join(flask::MANIFEST_NAME),
        "#settings\nmodule: 'app'\nversion: 0.0.0\n\n#dependencies\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(root.join("program"))
        .arg(root)
        .output()
        .unwrap();
    (directory, output)
}

/// An unknown name in a clobber list is ignored by LLVM. As an output it cannot build.
#[test]
fn a_register_the_target_does_not_have_is_a_build_failure() {
    let (directory, output) = build_with_target(&TARGET.replacen("'x1'", "'x99'", 1));
    assert!(!output.status.success(), "{output:?}");
    assert!(!directory.path().join("program").exists());
}

/// `~{x30}` was silently ignored, so the link register went unsaved and the program hung.
/// Spelled as an output, `x30` is the link register: it is saved and the program returns.
#[test]
fn the_link_register_is_saved_under_either_spelling() {
    let (directory, output) = build_with_target(&TARGET.replacen("'lr'", "'x30'", 1));
    assert!(output.status.success(), "{output:?}");
    let mut program = Command::new(directory.path().join("program"))
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = program.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            program.kill().unwrap();
            panic!("the program did not return");
        }
        std::thread::yield_now();
    };
    assert_eq!(status.code(), Some(0));
}

/// The `when` runs: `announce` holds an instruction, so it is a real call that is passed a
/// tag and chooses among arms at run time.
#[test]
fn a_union_chooses_at_run_time() {
    let (output, mlir) = run("\
#entry
start() in 0 to 255:
    announce(Mode.Loud)
    announce(Mode.Plain)
    announce(Mode.Quiet)
    return 0
");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "LPQ");
    assert!(mlir.contains("arith.select"), "{mlir}");
}

/// Changed: this expected a refusal, from when every arm of a `when` was computed together. An
/// arm that runs an instruction is now lowered into a branch of an `scf.if`, so it runs only when
/// taken and the build succeeds. The program is run with `Loud`: only `L` is printed (the
/// `Quiet` arm's `Q` would show if arms ran eagerly), and the exit status is the arm's value.
#[test]
fn an_arm_that_runs_an_instruction_runs_only_when_taken() {
    let (output, mlir) = run("\
chatty(mode Mode) in 0 to 255: when mode is
    Loud then announce(Mode.Loud)
    Quiet then announce(Mode.Quiet)
    Plain then 0

#entry
start() in 0 to 255: return chatty(Mode.Loud)
");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "L");
    assert_eq!(output.status.code(), Some(0));
    assert!(mlir.contains("scf.if"), "{mlir}");
    let (output, _) = run("\
chatty(mode Mode) in 0 to 255: when mode is
    Loud then announce(Mode.Loud)
    Quiet then announce(Mode.Quiet)
    Plain then 0

#entry
start() in 0 to 255: return chatty(Mode.Plain)
");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}
