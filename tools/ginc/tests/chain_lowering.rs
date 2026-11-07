//! A condition chain lowers to MLIR and the lowered function computes what the source says.
//!
//! `return when` with ordered arms is a nest of selects with the first arm outermost, or of
//! `scf.if` regions when an arm runs an instruction, so that an arm runs only when it is the
//! first whose condition holds.
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn fixture(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, text).unwrap();
    (directory, path)
}

fn emit(path: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg("--emit")
        .arg("mlir")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// The module between the ```mlir fence of `ginc`'s output.
fn module(output: &str) -> String {
    let start = output.find("module {").unwrap();
    let end = output.rfind("\n}").unwrap() + 2;
    output[start..end].to_owned()
}

/// Run the lowered `function` for each `n` in `0..=9`, passing `n` as the number of arguments
/// to `lli` so that nothing is known at compile time.
fn run_every_input(source: &str, function: &str, expected: impl Fn(i32) -> i32) {
    run_every_input_with(
        source,
        function,
        &[
            "--convert-arith-to-llvm",
            "--convert-func-to-llvm",
            "--reconcile-unrealized-casts",
        ],
        expected,
    );
}

fn run_every_input_with(
    source: &str,
    function: &str,
    pipeline: &[&str],
    expected: impl Fn(i32) -> i32,
) {
    let (_directory, path) = fixture(source);
    let lowered_module = module(&emit(&path));
    let wrapper = lowered_module.replacen(
        "module {",
        &format!("module {{\n  func.func @main(%argc: i32) -> i32 {{\n    %one = arith.constant 1 : i32\n    %count = arith.subi %argc, %one : i32\n    %n = arith.extsi %count : i32 to i64\n    %r = func.call @{function}(%n) : (i64) -> i64\n    %out = arith.trunci %r : i64 to i32\n    return %out : i32\n  }}"),
        1,
    );
    let mut arguments = pipeline.to_vec();
    if !arguments.contains(&"--reconcile-unrealized-casts") {
        arguments.push("--reconcile-unrealized-casts");
    }
    let lowered = pipe("mlir-opt", &arguments, &wrapper);
    let llvm = pipe("mlir-translate", &["--mlir-to-llvmir"], &lowered);
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("lowered.ll");
    std::fs::write(&file, llvm).unwrap();
    for n in 0..=9i32 {
        let arguments = vec!["x"; n as usize];
        let status = Command::new("lli")
            .arg(&file)
            .args(&arguments)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(expected(n)), "{function}({n})");
    }
}

fn pipe(program: &str, arguments: &[&str], input: &str) -> String {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{program}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

const GRADE: &str = "\
grade(n in 0 to 9) in 0 to 3:
    return when
        n < 3 then 0
        n < 6 then 1
        n < 8 then 2
        else 3
";

#[test]
fn a_chain_is_nested_selects_with_the_first_arm_outermost() {
    let (_directory, path) = fixture(GRADE);
    let text = emit(&path);
    assert_eq!(text.matches("arith.select").count(), 3, "{text}");
    assert!(!text.contains("scf.if"), "{text}");
    // The last select is the first arm: it chooses between 0 and the later arms' result.
    let last = text.rfind("arith.select").unwrap();
    assert!(text[last..].contains("%const1"), "{text}");
}

#[test]
fn the_lowered_chain_agrees_with_the_source_for_every_input() {
    run_every_input(GRADE, "grade", |n| match n {
        0..=2 => 0,
        3..=5 => 1,
        6..=7 => 2,
        _ => 3,
    });
}

#[test]
fn a_chain_over_different_operands() {
    let source = "\
pick(n in 0 to 9) in 0 to 9:
    return when
        n = 4 then 9
        n > 6 then n - 7
        n >= 2 then n
        else 5
";
    run_every_input(source, "pick", |n| {
        if n == 4 {
            9
        } else if n > 6 {
            n - 7
        } else if n >= 2 {
            n
        } else {
            5
        }
    });
}

#[test]
fn a_later_arm_value_uses_the_facts_of_the_failed_conditions() {
    // `n - 3` needs `n >= 3`, which the failed first condition gives.
    let source = "\
shift(n in 0 to 9) in 0 to 6:
    return when
        n < 3 then 0
        else n - 3
";
    run_every_input(source, "shift", |n| if n < 3 { 0 } else { n - 3 });
}

#[test]
fn a_chain_in_a_local_before_a_return() {
    let source = "\
band(n in 0 to 9) in 0 to 4:
    base in 0 to 2: when
        n < 3 then 0
        n < 6 then 1
        else 2
    return base + base
";
    run_every_input(source, "band", |n| {
        let base = if n < 3 {
            0
        } else if n < 6 {
            1
        } else {
            2
        };
        base + base
    });
}

/// Reserved registers none of the programs touch, and one instruction that traps if executed.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TRAP: &str = "brk #0";
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
const TRAP: &str = "ud2";

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn package(main: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        "mod.flask",
        "#settings\nmodule: \"chain\"\nversion: 0.1.0\n",
    );
    write(directory.path(), "main.gin", main);
    directory
}

fn prelude() -> String {
    format!(
        "#unrestricted\nNothing has\n\n#instruction('{TRAP}')\ntrap(input Type, output Type, inputs input) output\n\n#unrestricted\nMode is Quiet or Loud or Off\n\npick(n in 0 to 9) Mode:\n    when n < 3 then return Mode.Quiet\n    when n < 6 then return Mode.Loud\n    return Mode.Off\n\nrisky() in 0 to 9:\n    trap(Nothing, Nothing, Nothing())\n    return 2\n\n"
    )
}

fn mlir(main: &str) -> std::process::Output {
    let directory = package(main);
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg("--emit")
        .arg("mlir")
        .arg(directory.path())
        .output()
        .unwrap()
}

/// Build and run the program; its exit status is the value `start` returns.
fn run(main: &str) -> Option<i32> {
    let directory = package(main);
    let program = directory.path().join("program");
    let built = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    Command::new(&program).status().unwrap().code()
}

fn chain_program(chain: &str, input: u32) -> String {
    format!(
        "{}choose(n in 0 to 9) in 0 to 9:\n    return when\n{chain}\n\n#entry\nstart() in 0 to 255: return choose({input})\n",
        prelude()
    )
}

/// The trap is in the first arm; the input takes the last. If the first arm ran eagerly the
/// program would die instead of exiting with 3.
#[test]
fn the_first_arm_runs_its_instruction_only_when_taken() {
    let chain = "        n < 5 then risky()\n        n < 8 then 1\n        else 3";
    let text = String::from_utf8(mlir(&chain_program(chain, 9)).stdout).unwrap();
    assert!(text.contains("scf.if"), "{text}");
    assert!(text.contains("llvm.inline_asm"), "{text}");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&chain_program(chain, 9)), Some(3));
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&chain_program(chain, 6)), Some(1));
}

#[test]
fn a_middle_arm_runs_its_instruction_only_when_taken() {
    let chain = "        n < 3 then 1\n        n < 6 then risky()\n        else 3";
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&chain_program(chain, 1)), Some(1));
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&chain_program(chain, 8)), Some(3));
    let text = String::from_utf8(mlir(&chain_program(chain, 1)).stdout).unwrap();
    assert!(text.contains("scf.if"), "{text}");
}

#[test]
fn the_else_runs_its_instruction_only_when_no_arm_holds() {
    let chain = "        n < 3 then 1\n        n < 6 then 2\n        else risky()";
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&chain_program(chain, 4)), Some(2));
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&chain_program(chain, 0)), Some(1));
    let text = String::from_utf8(mlir(&chain_program(chain, 0)).stdout).unwrap();
    assert!(text.contains("scf.if"), "{text}");
}

/// A chain of variant tests: the arm for a member runs only when the subject is that member.
#[test]
fn a_variant_arm_runs_its_instruction_only_for_its_member() {
    let chain = |input: u32| {
        format!(
            "{}choose(n in 0 to 9) in 0 to 9:\n    return when\n        pick(n) is Quiet then 1\n        pick(n) is Loud then risky()\n        else 3\n\n#entry\nstart() in 0 to 255: return choose({input})\n",
            prelude()
        )
    };
    let text = String::from_utf8(mlir(&chain(1)).stdout).unwrap();
    assert!(text.contains("llvm.inline_asm"), "{text}");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        assert_eq!(run(&chain(1)), Some(1));
        assert_eq!(run(&chain(8)), Some(3));
    }
}

/// A condition joining several tests (`a and b`) is one comparison of a count of the tests
/// that hold, so the arm and the rest of the chain each own their nodes and lower.
#[test]
fn a_joined_condition_is_one_comparison_and_lowers() {
    let chain = "        n < 3 and n > 0 then 1\n        else 3";
    let program = chain_program(chain, 1);
    let output = mlir(&program);
    assert!(output.status.success(), "{output:?}");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        assert_eq!(run(&chain_program(chain, 1)), Some(1));
        // Negative controls: either test failing takes `else`.
        assert_eq!(run(&chain_program(chain, 0)), Some(3));
        assert_eq!(run(&chain_program(chain, 5)), Some(3));
    }
}
