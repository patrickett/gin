//! An arm that runs an instruction runs it only when the arm is taken.
//!
//! The recorded body is a value graph whose nodes all run, in order. Where a guard, a one-armed
//! case or a value conditional chooses between arms and one runs an instruction, the arm's nodes
//! are lowered into a branch of an `scf.if`. The programs below put a trapping instruction in the
//! arm that must not run: if it ran eagerly, the program would die instead of exiting with the
//! value of the other arm.
use std::path::Path;
use std::process::Command;

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
        "#settings\nmodule: \"lazy\"\nversion: 0.1.0\n",
    );
    write(directory.path(), "main.gin", main);
    directory
}

fn prelude() -> String {
    format!(
        "#unrestricted\nNothing has\n\n#instruction('{TRAP}')\ntrap(input Type, output Type, inputs input) output\n\n"
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

#[test]
fn a_guards_rest_runs_only_when_the_guard_fails() {
    let source = format!(
        "{}guard(n in 0 to 9) in 0 to 9:\n    when n < 5 then return 1\n    trap(Nothing, Nothing, Nothing())\n    return 2\n\n#entry\nstart() in 0 to 255: return guard(3)\n",
        prelude()
    );
    let text = String::from_utf8(mlir(&source).stdout).unwrap();
    // The instruction is inside a branch, not before the select.
    let branch = text.find("scf.if").expect(&text);
    let instruction = text.find("llvm.inline_asm").expect(&text);
    assert!(branch < instruction, "{text}");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&source), Some(1));
}

#[test]
fn the_guard_that_returns_does_not_run_the_trap_but_the_other_path_can() {
    // Guarded the other way: the trap is in the arm taken when the guard holds, and is not run
    // for an input where it does not.
    let source = format!(
        "{}guard(n in 0 to 9) in 0 to 9:\n    when n > 5 then\n        trap(Nothing, Nothing, Nothing())\n        return 1\n    return 2\n\n#entry\nstart() in 0 to 255: return guard(3)\n",
        prelude()
    );
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&source), Some(2));
    let text = String::from_utf8(mlir(&source).stdout).unwrap();
    assert!(text.contains("scf.if"), "{text}");
}

#[test]
fn a_one_armed_case_runs_its_rest_only_when_the_pattern_does_not_match() {
    let source = format!(
        "{}#unrestricted\nMode is Quiet or Loud\n\npick(n in 0 to 9) Mode:\n    when n < 5 then return Mode.Quiet\n    return Mode.Loud\n\nguard(n in 0 to 9) in 0 to 9:\n    when pick(n) is Quiet then return 1\n    trap(Nothing, Nothing, Nothing())\n    return 2\n\n#entry\nstart() in 0 to 255: return guard(3)\n",
        prelude()
    );
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&source), Some(1));
    let text = String::from_utf8(mlir(&source).stdout).unwrap();
    assert!(text.contains("scf.if"), "{text}");
}

#[test]
fn a_value_conditional_calls_an_effectful_function_only_in_its_arm() {
    let source = format!(
        "{}risky() in 0 to 9:\n    trap(Nothing, Nothing, Nothing())\n    return 2\n\nchoose(n in 0 to 9) in 0 to 9: return when n < 5 then 1 else risky()\n\n#entry\nstart() in 0 to 255: return choose(3)\n",
        prelude()
    );
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&source), Some(1));
    let text = String::from_utf8(mlir(&source).stdout).unwrap();
    assert!(text.contains("scf.if"), "{text}");
}

/// Changed: this expected a refusal of an `and` chain with an instruction in a later arm. The
/// instruction is in the arm taken when the chain is false, so it goes in the `scf.if`'s else
/// branch and runs only then. With `guard(3)` the chain is true: the program exits 1 and the trap
/// does not run (it would kill the program if it ran eagerly). `guard(0)` takes the else arm.
#[test]
fn a_chain_whose_other_arm_runs_an_instruction_runs_it_only_there() {
    let source = format!(
        "{}guard(n in 0 to 9) in 0 to 9:\n    when n > 1 and n < 5 then return 1\n    trap(Nothing, Nothing, Nothing())\n    return 2\n\n#entry\nstart() in 0 to 255: return guard(3)\n",
        prelude()
    );
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(&source), Some(1));
    let output = mlir(&source);
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let trap = text
        .find("brk #0")
        .or_else(|| text.find("ud2"))
        .expect(&text);
    let branch = text.find("scf.if").expect(&text);
    let otherwise = text.find("} else {").expect(&text);
    assert!(branch < otherwise && otherwise < trap, "{text}");
}

/// Pure arms keep the cheap branch-free form.
#[test]
fn pure_arms_are_still_a_select() {
    let source = "guard(n in 0 to 9) in 0 to 9:\n    when n < 5 then return 1\n    return 2\n\n#entry\nstart() in 0 to 255: return guard(3)\n";
    let text = String::from_utf8(mlir(source).stdout).unwrap();
    assert!(!text.contains("scf.if"), "{text}");
}
