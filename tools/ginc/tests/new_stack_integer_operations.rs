//! Division, remainder, bit operations and shifts lower to real artifacts and compute what the
//! source says for every input, under `lli`; and the ones the machine could not run safely are
//! refused rather than lowered.
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn fixture(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, text).unwrap();
    (directory, path)
}

fn emit(path: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg("--emit")
        .arg("mlir")
        .arg(path)
        .output()
        .unwrap()
}

fn mlir(text: &str) -> String {
    let (_directory, path) = fixture(text);
    let output = emit(&path);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
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

/// Run the lowered `function` for each `n` in `0..=9` under `lli`, the input being the number of
/// arguments so nothing is known at compile time, and compare with `expected`.
fn run_every_input(source: &str, function: &str, expected: impl Fn(i64) -> i64) {
    let text = mlir(source);
    let start = text.find("module {").unwrap();
    let end = text.rfind("\n}").unwrap() + 2;
    let wrapper = text[start..end].replacen(
        "module {",
        &format!("module {{\n  func.func @main(%argc: i32) -> i32 {{\n    %one = arith.constant 1 : i32\n    %count = arith.subi %argc, %one : i32\n    %n = arith.extsi %count : i32 to i64\n    %r = func.call @{function}(%n) : (i64) -> i64\n    %out = arith.trunci %r : i64 to i32\n    return %out : i32\n  }}"),
        1,
    );
    let lowered = pipe(
        "mlir-opt",
        &[
            "--convert-scf-to-cf",
            "--convert-arith-to-llvm",
            "--convert-cf-to-llvm",
            "--convert-func-to-llvm",
            "--reconcile-unrealized-casts",
        ],
        &wrapper,
    );
    let llvm = pipe("mlir-translate", &["--mlir-to-llvmir"], &lowered);
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("lowered.ll");
    std::fs::write(&file, llvm).unwrap();
    for n in 0..=9i64 {
        let status = Command::new("lli")
            .arg(&file)
            .args(vec!["x"; n as usize])
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(expected(n) as i32), "{function}({n})");
    }
}

#[test]
fn division_and_remainder_emit_signed_operations_and_compute_every_input() {
    let source = "d(n in 0 to 9) in 0 to 9: return n / 2\nr(n in 0 to 9) in 0 to 4: return n % 5\nneg(n in 0 to 9) in 0 to 20:\n    k in 0 to 9: n\n    return 10 + (k - 5) / 2\n";
    let text = mlir(source);
    assert!(
        text.contains("arith.divsi") && text.contains("arith.remsi"),
        "{text}"
    );
    run_every_input(source, "d", |n| n / 2);
    run_every_input(source, "r", |n| n % 5);
    // Truncation toward zero for a negative dividend: (n - 5) / 2 in Rust is the same.
    run_every_input(source, "neg", |n| 10 + (n - 5) / 2);
}

#[test]
fn the_bit_operations_and_shifts_compute_every_input() {
    let source = "b(n in 0 to 9) in 0 to 15: return (n & 5) | (n ^ 3)\nl(n in 0 to 9) in 0 to 72: return n << 3\nr(n in 0 to 9) in 0 to 9: return n >> 1\n";
    let text = mlir(source);
    for op in [
        "arith.andi",
        "arith.ori",
        "arith.xori",
        "arith.shli",
        "arith.shrsi",
    ] {
        assert!(text.contains(op), "{op}: {text}");
    }
    run_every_input(source, "b", |n| (n & 5) | (n ^ 3));
    run_every_input(source, "l", |n| n << 3);
    run_every_input(source, "r", |n| n >> 1);
}

/// The point of treating a division as something that must not be speculated: a guard that
/// excludes a zero divisor must stop the division from running, so the lowering puts it in a
/// branch. If it were computed beside the select (as a pure arm is), `lli` would trap on n = 0.
#[test]
fn a_guarded_division_never_runs_with_a_zero_divisor() {
    let source = "g(n in 0 to 9) in 0 to 9:\n    when n = 0 then return 7\n    return 9 / n\n";
    let text = mlir(source);
    assert!(
        text.contains("scf.if"),
        "the division is in a branch: {text}"
    );
    run_every_input(source, "g", |n| if n == 0 { 7 } else { 9 / n });
    let value = "v(n in 0 to 9) in 0 to 9: return when n = 0 then 7 else 9 / n\n";
    assert!(mlir(value).contains("scf.if"));
    run_every_input(value, "v", |n| if n == 0 { 7 } else { 9 / n });
}

#[test]
fn a_function_that_divides_is_not_speculated_either() {
    let source = "inv(n in 1 to 9) in 0 to 9: return 9 / n\nh(n in 0 to 9) in 0 to 9:\n    when n = 0 then return 7\n    return inv(n)\n";
    let text = mlir(source);
    assert!(text.contains("scf.if"), "the call is in a branch: {text}");
    run_every_input(source, "h", |n| if n == 0 { 7 } else { 9 / n });
}

// ---- negative controls: refused, not lowered ------------------------------------------------

fn refused(source: &str) {
    let (_directory, path) = fixture(source);
    let output = emit(&path);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("func.func"),
        "{output:?}"
    );
}

#[test]
fn a_shift_whose_result_does_not_fit_a_register_is_refused() {
    // Exact at the checker's width, but no i64: the lowering fails closed.
    refused("big(n in 0 to 9) in 0 to 100000000000000000000000000000: return n << 70\n");
}

#[test]
fn a_shift_amount_the_machine_could_not_take_is_refused() {
    // The checker accepts amounts to 254; a shift by 64 or more bits is undefined in the machine.
    refused("z(m in 0 to 100) in 0 to 0: return 0 << m\n");
}

/// A chain of `and` tests used to share its arms, so an arm that divides was refused. The tests
/// are now one count (each is 1 or 0), so the arm is in a branch of its own and runs only when the
/// whole chain holds; every input agrees with the checker, including the ones the chain excludes.
#[test]
fn a_division_in_an_arm_a_chain_guards_runs_only_when_the_chain_holds() {
    let source =
        "c(n in 0 to 9) in 0 to 9:\n    when n > 0 and n < 8 then return 9 / n\n    return 1\n";
    let text = mlir(source);
    assert!(
        text.contains("scf.if"),
        "the division is in a branch: {text}"
    );
    run_every_input(source, "c", |n| if n > 0 && n < 8 { 9 / n } else { 1 });
    // An `or` chain, and a membership test, the same.
    let either =
        "e(n in 0 to 9) in 0 to 9:\n    when n = 0 or n > 7 then return 1\n    return 9 / n\n";
    run_every_input(either, "e", |n| if n == 0 || n > 7 { 1 } else { 9 / n });
    let within =
        "w(n in 0 to 9) in 0 to 9:\n    when n is in 1 to 7 then return 9 / n\n    return 2\n";
    run_every_input(
        within,
        "w",
        |n| if (1..=7).contains(&n) { 9 / n } else { 2 },
    );
}
