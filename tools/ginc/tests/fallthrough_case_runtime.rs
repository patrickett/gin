//! A one-armed `when x is Member then <effects>` that falls through, built and run natively: the
//! effects of the arm run only when the subject is the member, and the statements after it run
//! either way.
use std::path::{Path, PathBuf};
use std::process::Command;

fn build(source: &str) -> Result<(tempfile::TempDir, PathBuf), String> {
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"fallthrough\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), source).unwrap();
    let program = directory.path().join("program");
    let built = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(directory.path())
        .output()
        .unwrap();
    if built.status.success() {
        Ok((directory, program))
    } else {
        Err(String::from_utf8_lossy(&built.stderr).into_owned())
    }
}

const PROGRAM: &str = "\
use core.io print_int

#unrestricted
Mode is Loud or Quiet

shout(mode Mode):
    print_int(1)
    when mode is Loud then
        print_int(2)
    print_int(3)
    return

#entry
main in 0 to 1:
    shout(Mode.Loud)
    shout(Mode.Quiet)
    return 0
";

#[test]
fn the_arm_runs_only_for_its_member_and_the_rest_runs_for_both() {
    let (_directory, program) = build(PROGRAM).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "1\n2\n3\n1\n3\n");
    assert_eq!(done.status.code(), Some(0));
}

#[test]
fn a_changed_arm_changes_the_output() {
    // Negative control: if the arm were always or never run the output would differ.
    let swapped = PROGRAM.replace("when mode is Loud then", "when mode is Quiet then");
    let (_directory, program) = build(&swapped).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "1\n3\n1\n2\n3\n");
}

const READ: &str = "\
use core.io read_int, print_int, ReadResult
use core.primitive SignedBigInt

shown(result ReadResult) in 0 to 1: when result is
    Value(_) then 0
    Unread(_) then 1

#entry
main in 0 to 1:
    got: read_int
    when got is Unread(error) then
        return 1
    when shown(got) = 1 then
        return 1
    Value(n) := got
    print_int(n)
    return 0
";

fn run_with_input(program: &Path, input: &str) -> (String, Option<i32>) {
    use std::io::Write;
    let mut child = Command::new(program)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let done = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&done.stdout).into_owned(),
        done.status.code(),
    )
}

#[test]
fn a_narrowed_imported_sum_is_passed_on_and_destructured() {
    let (_directory, program) = build(READ).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(run_with_input(&program, "41"), ("41\n".to_owned(), Some(0)));
    // Negative control: no number is the guard's return, so nothing is printed.
    assert_eq!(run_with_input(&program, "x"), (String::new(), Some(1)));
}

#[test]
fn read_int_holds_the_64_bit_edges_through_valued_chains() {
    // `io.accumulate` and `io.passes` are valued chains whose arms span the 64-bit edge.
    let (_directory, program) = build(READ).unwrap_or_else(|e| panic!("{e}"));
    for ok in ["9223372036854775807", "-9223372036854775808"] {
        assert_eq!(run_with_input(&program, ok), (format!("{ok}\n"), Some(0)));
    }
    // Negative control: one past either edge is an overflow, which prints nothing.
    for over in ["9223372036854775808", "-9223372036854775809"] {
        assert_eq!(run_with_input(&program, over), (String::new(), Some(1)));
    }
}

// Inline result variants: recorded, lowered and run.

const INLINE: &str = "\
use core.io print_int

#unrestricted
Code is in 0 to 9

check(n in 0 to 9) Fine or Failed(Code):
    when n < 5 then
        return Fine
    return Failed(n as Code)

report(n in 0 to 9):
    got: check(n)
    when got is Failed(_) then
        print_int(2)
        return
    print_int(1)
    return

#entry
main in 0 to 1:
    report(3)
    report(7)
    return 0
";

#[test]
fn a_function_with_inline_result_variants_runs_natively() {
    let (_directory, program) = build(INLINE).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "1\n2\n");
    // Negative control: the other branch for the other input.
    let swapped = INLINE.replace("report(3)\n    report(7)", "report(7)\n    report(3)");
    let (_directory, program) = build(&swapped).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "2\n1\n");
}

// Named payload fields: several, built by label and taken out by position or label, natively.

const PAIR: &str = "\
#unrestricted
Cell has
    value in 0 to 9

make(n in 0 to 9) Both(first Cell, second Cell) or Neither:
    when n > 5 then
        return Neither
    return Both(first: Cell(value: n), second: Cell(value: 9))

#entry
main in 0 to 18:
    return when make(MAIN_ARGUMENT) is
        Both(a, b) then a.value + b.value
        Neither then 0
";

#[test]
fn several_named_payloads_round_trip_natively() {
    for (argument, expected) in [("3", 12), ("7", 0), ("0", 9)] {
        let source = PAIR.replace("MAIN_ARGUMENT", argument);
        let (_directory, program) = build(&source).unwrap_or_else(|e| panic!("{argument}: {e}"));
        let done = Command::new(&program).output().unwrap();
        assert_eq!(done.status.code(), Some(expected), "{argument}");
    }
    // Labeled patterns take the same payloads.
    let labeled = PAIR.replace("MAIN_ARGUMENT", "3").replace(
        "Both(a, b) then a.value + b.value",
        "Both(x: second, y: first) then x.value + y.value",
    );
    let (_directory, program) = build(&labeled).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        Command::new(&program).output().unwrap().status.code(),
        Some(12)
    );
}
