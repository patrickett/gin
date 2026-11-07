//! New-stack emission: parse -> check -> lower -> MLIR text, with no legacy path.
use std::path::{Path, PathBuf};
use std::process::Command;

use ginc::cli::{Args, Emit};
use ginc::compile::{CompileResult, GinCompiler};
use ginc::reporting::CheckOptions;

const VALID: &str = include_str!("../../../crates/typecheck/examples/source-echo.gin");

fn fixture(text: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, text).unwrap();
    (directory, path)
}

fn command(path: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
    command
        .arg("--emit")
        .arg("mlir")
        .arg(path)
        .env_remove("GIN_PROFILE_JSON")
        .env_remove("GINC_TIMINGS")
        .env_remove("GINC_FAST_LLVM");
    command
}

fn files(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn emits_verified_mlir_for_a_real_new_source_fixture() {
    let (directory, path) = fixture(VALID);
    let before = files(directory.path());
    let output = command(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("func.func @echo"), "{stdout}");
    assert!(stdout.contains("return %arg0 : i64"), "{stdout}");
    assert!(
        stdout.trim_start().starts_with("```mlir\nmodule {"),
        "{stdout}"
    );
    assert_eq!(files(directory.path()), before);
}

#[test]
fn emits_an_integer_constant_body() {
    let (_directory, path) = fixture("answer() in 0 to 5: return 3\n");
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("func.func @answer"), "{stdout}");
    assert!(stdout.contains("arith.constant 3 : i64"), "{stdout}");
}

#[test]
fn emits_a_function_that_reassigns_and_loops() {
    // Mutation and unrolled loops are recorded as plain dataflow, so they lower with no
    // new operations: the function is arithmetic on its argument.
    let source = "total(a in 0 to 9) in 0 to 99:\n  s in 0 to 99: a\n  for i in 1 to 3 do\n    s +: i\n  s *: 2\n  return s\n";
    let (_directory, path) = fixture(source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("func.func @total"), "{stdout}");
    assert!(stdout.contains("arith.addi"), "{stdout}");
    assert!(stdout.contains("arith.muli"), "{stdout}");
}

#[test]
fn a_loop_of_unknown_count_lowers_to_an_scf_for_carrying_its_locals() {
    let source = "total(n in 0 to 9) in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to n do\n    s +: i\n  return s\n";
    let (_directory, path) = fixture(source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("scf.for"), "{stdout}");
    assert!(stdout.contains("iter_args"), "{stdout}");
    assert!(stdout.contains("scf.yield"), "{stdout}");
    assert!(stdout.contains("arith.addi"), "{stdout}");
}

#[test]
fn nested_loops_with_several_carried_values_lower_to_nested_regions() {
    let source = "f(n in 0 to 4) in 0 to 999:\n  a in 0 to 400: 0\n  b in 0 to 400: 0\n  for i in 1 to n do\n    for j in 1 to i do\n      a +: 1\n    b +: i\n  return a + b\n";
    let (_directory, path) = fixture(source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.matches("scf.for").count(), 2, "{stdout}");
    assert!(stdout.contains("-> (i64, i64)"), "{stdout}");
}

#[test]
fn a_mut_number_is_handed_back_as_an_extra_result() {
    let source = "bump(x mut in 0 to 9, by in 0 to 3) in 0 to 9:\n  x :: when x < 6 then x + by else x\n  return by\nf(a in 0 to 6) in 0 to 18:\n  n in 0 to 9: a\n  r: bump(n, 2)\n  return n + r\n";
    let (_directory, path) = fixture(source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The callee returns its result and then the new value of `x`; the caller reads both.
    assert!(stdout.contains("-> (i64, i64)"), "{stdout}");
    assert!(stdout.contains("%call"), "{stdout}");
    assert!(stdout.contains("#0") && stdout.contains("#1"), "{stdout}");
}

#[test]
fn a_function_that_only_changes_a_number_returns_just_that_number() {
    let source = "reset(x mut in 0 to 9):\n  x :: 0\n  return\nf(a in 0 to 9) in 0 to 9:\n  n in 0 to 9: a\n  reset(n)\n  return n\n";
    let (_directory, path) = fixture(source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("func.func @reset(%arg0: i64) -> i64"),
        "{stdout}"
    );
}

#[test]
fn emits_a_call_to_an_associated_function_of_a_type() {
    let source = concat!(
        "Point has\n",
        "  x in 0 to 9\n",
        "  add(a in 0 to 5, b in 0 to 5) in 0 to 10: return a + b\n",
        "go(a in 0 to 5) in 0 to 10: return Point.add(a, 2)\n",
    );
    let (_directory, path) = fixture(source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("func.func @Point.add"), "{stdout}");
    assert!(stdout.contains("func.call @Point.add"), "{stdout}");
}

#[test]
fn record_receivers_fail_closed_until_records_lower() {
    let source = concat!(
        "#unrestricted\n",
        "Point has\n",
        "  x in 0 to 9\n",
        "  keep(self): return\n",
        "go(p Point): p.keep\n",
        "  return\n",
    );
    let (_directory, path) = fixture(source);
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("func.func"));
}

#[test]
fn unrecorded_bodies_fail_closed_without_a_legacy_fallback() {
    let (_directory, path) = fixture(
        "#unrestricted\nP has x in 0 to 9\nanswer(n in 0 to 3) in 0 to 9:\n  q: P(x: 1)\n  for i in 0 to n do\n    q.x :: 2\n  return q.x\n",
    );
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("no executable body was recorded") || stderr.contains("lowering failed"),
        "{output:?}"
    );
    assert!(!stdout.contains("func.func"), "{stdout}");
}

#[test]
fn build_options_are_rejected_when_emitting_mlir() {
    let (directory, path) = fixture(VALID);
    let options: &[&[&str]] = &[
        &["--emit", "exe"],
        &["--profile", "release"],
        &["--target", "invalid-target"],
        &["--no-cache"],
        &["--fast-llvm"],
    ];
    for options in options {
        let output = command(&path)
            .current_dir(directory.path())
            .args(*options)
            .output()
            .unwrap();
        // MLIR text has no profile, cache or linker; `--output` and `--timings` are accepted.
        let code = output.status.code();
        assert!(matches!(code, Some(1 | 2)), "{options:?}: {output:?}");
        assert_eq!(files(directory.path()), ["input.gin"]);
    }
}

#[test]
fn programmatic_entry_points_emit_without_writing_artifacts() {
    let (directory, path) = fixture(VALID);
    for entry in 0..3 {
        let mut args = Args {
            input: path.clone(),
            emit: Emit::Mlir,
            ..Default::default()
        };
        let result = match entry {
            0 => GinCompiler::compile(&mut args),
            1 => GinCompiler::compile_cli(&mut args),
            2 => GinCompiler::check(&mut args, CheckOptions::default()),
            _ => unreachable!(),
        };
        assert!(matches!(result, CompileResult::Success { output: None }));
        assert_eq!(files(directory.path()), ["input.gin"]);
    }
}

const MANIFEST: &str = "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n";

fn package(files: &[(&str, &str)]) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("mod.flask"), MANIFEST).unwrap();
    for (path, text) in files {
        let path = directory.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    directory
}

#[test]
fn a_two_module_package_emits_one_mlir_module_per_gin_module() {
    let directory = package(&[
        (
            "main.gin",
            "use app.util id\nwrapped(x in 0 to 16) in 0 to 16: return id(x)\n",
        ),
        (
            "util/util.gin",
            "five() in 0 to 16: return 5\nid(x in 0 to 16) in 0 to 16: return x\n",
        ),
    ]);
    let before = files(directory.path());
    let output = command(directory.path()).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let util = stdout.find("// gin module: app.util").expect("util block");
    let app = stdout.find("// gin module: app\n").expect("app block");
    assert!(util < app, "dependencies are emitted first: {stdout}");
    assert_eq!(stdout.matches("```mlir").count(), 2, "{stdout}");
    assert!(
        stdout.contains("func.func @\"app.util.five\"() -> i64 {"),
        "{stdout}"
    );
    assert!(
        stdout.contains("func.func private @\"app.util.id\"(i64) -> i64"),
        "{stdout}"
    );
    assert!(
        stdout.contains("func.call @\"app.util.id\"(%arg0)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("func.func @\"app.wrapped\"(%arg0: i64) -> i64 {"),
        "{stdout}"
    );
    assert_eq!(files(directory.path()), before);
}

#[test]
fn a_package_with_an_unlowerable_body_fails_closed() {
    let directory = package(&[(
        "main.gin",
        "#unrestricted\nP has x in 0 to 9\n\
         bound(n in 0 to 3) in 0 to 9:\n  q: P(x: 1)\n  for i in 0 to n do\n    q.x :: 2\n  return q.x\n",
    )]);
    let output = command(directory.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("func.func"), "{stdout}");
}

#[test]
fn package_emission_reports_check_failures_without_emitting() {
    let directory = package(&[("main.gin", "use app.util missing\n")]);
    let output = command(directory.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("```mlir"));
}

fn exe_command(directory: &Path, output: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
    command
        .args(["--emit", "exe", "--output"])
        .arg(output)
        .arg(directory)
        .env_remove("GIN_PROFILE_JSON")
        .env_remove("GINC_TIMINGS")
        .env_remove("GINC_FAST_LLVM");
    command
}

const UTIL: &str = "five() in 0 to 16: return 5\nid(x in 0 to 16) in 0 to 16: return x\n";

#[test]
fn a_two_module_package_links_into_an_executable_with_the_entry_status() {
    let directory = package(&[
        (
            "main.gin",
            "use app.util id\nuse app.util five\n#entry\nstart() in 0 to 16: return id(five())\n",
        ),
        ("util/util.gin", UTIL),
    ]);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let status = Command::new(&program).status().unwrap();
    assert_eq!(status.code(), Some(5));
}

#[test]
fn a_unit_entry_exits_zero() {
    let directory = package(&[("main.gin", "#entry\nstart():\n    return\n")]);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(Command::new(&program).status().unwrap().code(), Some(0));
}

const NUMERIC: &str = "\
Numeric has
    #infix('+')
    add(self, rhs Self):
        a in Self: self
        b in Self: rhs
        return a + b
";

/// A `Byte` domain with `+`, a way to read it back, and the given entry arithmetic.
fn byte_program(markers: &str, entry_body: &str) -> String {
    // Only a wrapping domain can prove `x + 1` stays inside it.
    let inc = if markers.contains("#wrap") {
        "inc(x Byte) Byte: return x + 1\n"
    } else {
        ""
    };
    format!(
        "{NUMERIC}{markers}\nByte is in 0 to 255\nByte has Numeric\n    value(self) in 0 to 255: self\n{inc}unwrap(x Byte) in 0 to 255: return x.value\n#entry\nstart() in 0 to 255: {entry_body}\n"
    )
}

fn exit_code(source: &str) -> Option<i32> {
    let directory = package(&[("main.gin", source)]);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    Command::new(&program).status().unwrap().code()
}

#[test]
fn a_domain_lowers_to_its_integer_and_runs() {
    let source = byte_program("#unrestricted", "return unwrap(42)");
    assert_eq!(exit_code(&source), Some(42));
}

#[test]
fn a_wrapping_domain_reduces_modulo_its_width_at_run_time() {
    let wrapping = |value: u32| {
        byte_program(
            "#unrestricted, #wrap",
            &format!("return unwrap(inc({value}))"),
        )
    };
    assert_eq!(exit_code(&wrapping(254)), Some(255));
    assert_eq!(exit_code(&wrapping(255)), Some(0));
    assert_eq!(exit_code(&wrapping(0)), Some(1));
}

#[test]
fn a_domain_program_emits_modular_arithmetic_only_where_it_wraps() {
    let (_directory, path) = fixture(&byte_program(
        "#unrestricted, #wrap",
        "return unwrap(inc(7))",
    ));
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("arith.remsi"), "{stdout}");
    let (_directory, path) = fixture(&format!(
        "{NUMERIC}#unrestricted\nByte is in 0 to 255\nByte has Numeric\nvalue(x Byte) Byte: return x\n"
    ));
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8(output.stdout).unwrap().contains("remsi"));
}

#[test]
fn a_domain_wider_than_the_i64_placeholder_fails_closed() {
    let source = format!(
        "{NUMERIC}#unrestricted, #wrap\nWord is in 0 to 18446744073709551615\nWord has Numeric\ninc(x Word) Word: return x + 1\n"
    );
    let (_directory, path) = fixture(&source);
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
}

#[test]
fn a_conditional_lowers_to_a_comparison_and_a_select() {
    let source = concat!(
        "max(a in 0 to 9, b in 0 to 9) in 0 to 9: return when a < b then b else a\n",
        "chain(a in 0 to 9) in 0 to 2:\n",
        "  return when a < 3 then 0 else when a < 6 then 1 else 2\n",
    );
    let (_directory, path) = fixture(source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("arith.cmpi slt"), "{stdout}");
    assert_eq!(stdout.matches("arith.select").count(), 3, "{stdout}");
}

#[test]
fn building_without_an_entry_fails_without_writing_an_executable() {
    let directory = package(&[("main.gin", "start() in 0 to 16: return 1\n")]);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(!program.exists());
}

#[test]
fn an_entry_outside_the_root_module_is_rejected() {
    let directory = package(&[
        (
            "main.gin",
            "use app.util five\n#entry\nstart() in 0 to 16: return five()\n",
        ),
        ("util/util.gin", "#entry\nfive() in 0 to 16: return 5\n"),
    ]);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(!program.exists());
}

#[test]
fn an_executable_goes_to_the_default_output_and_needs_a_package_directory() {
    let directory = package(&[("main.gin", "#entry\nstart():\n    return\n")]);
    let without_output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe"])
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(without_output.status.success(), "{without_output:?}");
    assert!(directory.path().join("target/debug/app").is_file());
    let (single, file) = fixture("#entry\nstart():\n    return\n");
    let program = single.path().join("program");
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(&file)
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(!program.exists());
}

#[test]
fn an_unknown_directive_is_reported_as_an_unknown_compiler_query() {
    let directory = package(&[("main.gin", "#inline\nstart():\n    return\n")]);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("unknown compiler query"), "{text}");
    assert!(!program.exists());
}

fn emitted(text: &str) -> String {
    let (_directory, path) = fixture(text);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn arithmetic_on_unknown_values_lowers_to_machine_operations() {
    let mlir = emitted(
        "sum(a in 0 to 100, b in 0 to 100) in 0 to 200: return a + b\n\
         diff(a in 0 to 100, b in 0 to 100) in -100 to 100: return a - b\n\
         scaled(a in 0 to 10) in 0 to 30: return a * 3\n",
    );
    assert!(mlir.contains("arith.addi %arg0, %arg1 : i64"), "{mlir}");
    assert!(mlir.contains("arith.subi %arg0, %arg1 : i64"), "{mlir}");
    assert!(mlir.contains("arith.muli %arg0, %const"), "{mlir}");
}

#[test]
fn operations_on_known_values_never_reach_the_output() {
    let mlir = emitted(
        "five() in 0 to 16: return 5\n\
         again() in 0 to 16: return five() + 1\n\
         constant() in 0 to 16: return 2 + 3 * 4\n",
    );
    // `again` is the constant 6 and `constant` is 14; neither calls nor computes.
    assert!(mlir.contains("arith.constant 6 : i64"), "{mlir}");
    assert!(mlir.contains("arith.constant 14 : i64"), "{mlir}");
    for forbidden in ["func.call", "arith.addi", "arith.muli"] {
        assert!(!mlir.contains(forbidden), "{forbidden} in {mlir}");
    }
}

/// Two numbers below 2^63 sum to a number below 2^64: held in an unsigned register, the machine
/// addition is exact. A sum past 2^64 has no register to be returned in, so it fails closed.
#[test]
fn arithmetic_is_exact_in_an_unsigned_register_and_fails_closed_past_it() {
    let (_directory, path) =
        fixture("wide(a in 0 to 9223372036854775807) in 0 to 18446744073709551614: return a + a\n");
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("arith.addi"));
    let (_directory, path) = fixture(
        "wide(a in 0 to 18446744073709551615) in 0 to 36893488147419103230: return a + a\n",
    );
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("func.func"));
}

#[test]
fn a_call_into_another_module_with_known_arguments_leaves_no_call_or_declaration() {
    let directory = package(&[
        (
            "main.gin",
            "use app.util id\nuse app.util five\n\
             answer() in 0 to 16: return id(five())\n",
        ),
        ("util/util.gin", UTIL),
    ]);
    let output = command(directory.path()).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let app = &stdout[stdout.find("// gin module: app\n").expect("app block")..];
    // The importer holds the value 5: no call, and so no private declaration to link.
    assert!(app.contains("arith.constant 5 : i64"), "{app}");
    for forbidden in ["func.call", "func.func private"] {
        assert!(!app.contains(forbidden), "{forbidden} in {app}");
    }
}

const PAIR: &str = "#unrestricted\nPair has\n  x in 0 to 9\n  y in 0 to 9\n";

#[test]
fn a_record_of_numbers_travels_as_registers() {
    let source = format!(
        "{PAIR}swap(p Pair) Pair: return Pair(x: p.y, y: p.x)\nf(a in 0 to 9, b in 0 to 9) in 0 to 9:\n  q: swap(Pair(x: a, y: b))\n  return q.x\n"
    );
    let (_directory, path) = fixture(&source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // One argument becomes two registers, and the result two.
    assert!(
        stdout.contains("@swap(%arg0_0: i64, %arg0_1: i64) -> (i64, i64)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("return %arg0_1, %arg0_0 : i64, i64"),
        "{stdout}"
    );
}

/// A record of `count` fields, passed through `id`.
fn wide_record(count: usize) -> String {
    let fields: String = (0..count).map(|n| format!("  f{n} in 0 to 9\n")).collect();
    format!("#unrestricted\nBig has\n{fields}id(x Big) Big: return x\n")
}

#[test]
fn a_record_up_to_sixteen_registers_travels_in_them_and_one_over_goes_by_address() {
    // Sixteen registers stay flattened; wider records travel by stack-slot address.
    let text = emitted(&wide_record(16));
    assert!(text.contains("%arg0_15: i64"), "{text}");
    // Seventeen no longer fail: the record is held in a stack slot and passed by its address
    // (option B of Q7, built). The exact values are round-tripped by
    // `a_record_wider_than_the_registers_round_trips_through_a_slot`.
    let text = emitted(&wide_record(17));
    assert!(
        text.contains("%arg0: i64, %sret0: i64")
            && text.contains("llvm.intr.memcpy")
            && !text.contains("%arg0_16"),
        "{text}"
    );
}

#[test]
fn a_mut_record_is_handed_back_as_its_registers() {
    let source = format!(
        "{PAIR}shift(p mut Pair):\n  p.x :: p.y\n  return\nf(a in 0 to 9, b in 0 to 9) in 0 to 9:\n  q: Pair(x: a, y: b)\n  shift(q)\n  return q.x\n"
    );
    let (_directory, path) = fixture(&source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("@shift(%arg0_0: i64, %arg0_1: i64) -> (i64, i64)"),
        "{stdout}"
    );
}

#[test]
fn a_mut_call_into_another_module_declares_the_extra_result() {
    let directory = package(&[
        (
            "main.gin",
            "use app.util reset\nf(a in 0 to 9) in 0 to 9:\n  n in 0 to 9: a\n  reset(n)\n  return n\n",
        ),
        (
            "util/util.gin",
            "reset(x mut in 0 to 9):\n  x :: 0\n  return\n",
        ),
    ]);
    let output = command(directory.path()).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("func.func private"), "{stdout}");
    assert!(stdout.contains("(i64) -> i64"), "{stdout}");
}

#[test]
fn a_load_and_a_store_lower_to_typed_llvm_memory_operations() {
    let source = "#unrestricted\nByte is in 0 to 255\n\n#load\nload(element Type, pointer ref @element, index in 0 to 15) element\n\n#store\nstore(element Type, pointer ref @element, index in 0 to 15, value element) ()\n\nswap(pointer ref @Byte, from in 0 to 15, into in 0 to 15):\n    value: load(Byte, pointer, from)\n    store(Byte, pointer, into, value)\n    return\n";
    let (_directory, path) = fixture(source);
    let output = command(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // A byte is eight bits in memory, widened to the register on a read and narrowed on a write.
    for expected in [
        "llvm.getelementptr",
        "-> !llvm.ptr, i8",
        "llvm.load %addr2 : !llvm.ptr -> i8",
        "arith.extui",
        "arith.trunci",
        "llvm.store %narrow5, %addr5 : i8, !llvm.ptr",
    ] {
        assert!(stdout.contains(expected), "{expected}\n{stdout}");
    }
    // A function that only checks (no `#load`) emits nothing of the kind.
    let (_directory, path) = fixture("answer() in 0 to 5: return 3\n");
    let stdout = String::from_utf8(command(&path).output().unwrap().stdout).unwrap();
    assert!(!stdout.contains("llvm.load") && !stdout.contains("llvm.store"));
}

/// The kernel-call pattern of `core.target.kernel` (a domain result chosen by the target, each arm
/// a hand-assembled instruction), written out so it is a fixture and not the library.
const KERNEL_PATTERN: &str = include_str!("fixtures/kernel_pattern.gin");

#[test]
fn a_target_chosen_domain_result_lowers_to_the_instruction_of_its_target() {
    let source = format!("{KERNEL_PATTERN}\n#entry\nmain:\n    got: send(1, 0, 0)\n    return\n");
    let directory = package(&[("main.gin", &source)]);
    let output = command(directory.path()).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The target's arm is the body: its inline assembly is there, the other targets' are not, and
    // `else 0` (a conversion into the domain on a path that never runs) left no trace.
    assert!(stdout.contains("llvm.inline_asm"), "{stdout}");
    let asm = stdout.matches("llvm.inline_asm").count();
    assert_eq!(
        asm, 2,
        "one arm per function (`receive` and `send`), the target's: {stdout}"
    );
    assert!(stdout.contains("func.func @\"app.send\""), "{stdout}");
    assert!(stdout.contains("func.func @\"app.receive\""), "{stdout}");
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn a_built_executable_reads_stdin_and_writes_it_back_through_the_wrappers() {
    // Run: a page is mapped (the Darwin instruction of `core.alloc.system`'s pattern), four bytes
    // are read into it and written back, through a file standing in for a pipe.
    let reserve = "ReserveIn has\n    #register('x9')\n    size Size\n\n#unrestricted\nReserveOut has\n    #register('x0')\n    value Address\n    #register('x1')\n    c0 Result\n    #register('x2')\n    c1 Result\n    #register('x3')\n    c2 Result\n    #register('x4')\n    c3 Result\n    #register('x5')\n    c4 Result\n    #register('x16')\n    c5 Result\n    #register('nzcv')\n    c6 Result\n\n#instruction('.byte 0, 0, 128, 210, 225, 3, 9, 170, 98, 0, 128, 210, 67, 0, 130, 210, 4, 0, 128, 146, 5, 0, 128, 210, 176, 24, 128, 210, 1, 16, 0, 212, 67, 0, 0, 84, 0, 0, 128, 210')\nreserve_darwin(input Type, output Type, inputs input) output\n";
    let source = format!(
        "{KERNEL_PATTERN}\n{reserve}\n#entry\nmain:\n    address: reserve_darwin(ReserveIn, ReserveOut, ReserveIn(size: 64)).value\n    got: receive(0, address, 16)\n    echoed: send(1, address, 4)\n    return\n"
    );
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), source).unwrap();
    let executable = directory.path().join("echo");
    let built = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&executable)
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    let input = directory.path().join("input.txt");
    std::fs::write(&input, "xyz\n").unwrap();
    let result = Command::new(&executable)
        .stdin(std::fs::File::open(&input).unwrap())
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"xyz\n");
}

// `deref x` of a borrowed number is a read of the parameter, and a statement before the final
// `return` (a call made for its effect) lowers.
const COPYING: &str = "\
#unrestricted
Small is in 0 to 255
Small has
    copy_of(self ref) Small: return deref self
    value(self) in 0 to 255: self
    release(self):
        return
";

#[test]
fn a_deref_of_a_ref_parameter_is_a_read_of_it() {
    let mlir = emitted(&format!(
        "{COPYING}twice(x Small) Small: return x.copy_of\n"
    ));
    assert!(
        mlir.contains("func.func @Small.copy_of(%arg0: i64) -> i64"),
        "{mlir}"
    );
    assert!(mlir.contains("return %arg0 : i64"), "{mlir}");
}

#[test]
fn a_copy_call_runs_natively() {
    let source = format!(
        "{COPYING}#entry\nstart() in 0 to 255:\n    a Small: 42\n    b: a.copy_of\n    return b.value\n"
    );
    assert_eq!(exit_code(&source), Some(42));
}

#[test]
fn a_call_for_its_effect_before_the_return_lowers() {
    let source = "note(x in 0 to 9):\n    return\npick(a in 0 to 9, b in 0 to 9) in 0 to 9:\n    note(a)\n    return b\n#entry\nstart() in 0 to 9: return pick(1, 6)\n";
    assert_eq!(exit_code(source), Some(6));
    // Control: a guard that returns early still picks the right result.
    let early = "pick(a in 0 to 9) in 0 to 9:\n    when a < 5 then\n        return 1\n    return 2\n#entry\nstart() in 0 to 9: return pick(7)\n";
    assert_eq!(exit_code(early), Some(2));
}

#[test]
fn a_deref_that_would_move_a_linear_value_is_refused() {
    let linear = "Token has\n    n in 0 to 9\n    take(self ref) Token: return deref self\n    end(self):\n        return\n";
    let (_directory, path) = fixture(linear);
    let output = command(&path).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
}

/// The draft `modules/core` library lowers and links whole, and a program over it runs.
#[test]
fn the_whole_core_package_lowers_and_links() {
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"probe\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("main.gin"),
        "#entry\nstart() in 0 to 9:\n    return 4\n",
    )
    .unwrap();
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(Command::new(&program).status().unwrap().code(), Some(4));
}

// A borrowed array is the address of its first element plus the sibling length: two registers
// for a view, and an element read is a typed load at `base + index * size`.
const VIEW: &str = "\
#unrestricted
Byte is in 0 to 255
Bytes has
    len in 0 to 1000
    data ref [Byte; len]
    at(self ref, index in 0 to 1000 and index < self.len) Byte:
        return deref self.data.[index]
    peek(self ref, index in 0 to 1000 and index < self.len) ref Byte:
        return ref self.data.[index]
";

#[test]
fn an_element_of_a_borrowed_array_lowers_to_a_typed_load() {
    let mlir = emitted(&format!(
        "{VIEW}first(b ref Bytes and b.len > 0) Byte: return b.at(0)\n"
    ));
    assert!(
        mlir.contains("(%arg0_0: i64, %arg0_1: i64, %arg1: i64)"),
        "{mlir}"
    );
    assert!(mlir.contains("llvm.getelementptr %base"), "{mlir}");
    assert!(mlir.contains("llvm.load %addr"), "{mlir}");
    assert!(mlir.contains("!llvm.ptr -> i8"), "{mlir}");
    let peeked = emitted(&format!(
        "{VIEW}first(b ref Bytes and b.len > 0) ref Byte: return b.peek(0)\n"
    ));
    assert!(peeked.contains("llvm.load"), "{peeked}");
}

#[test]
fn an_unproved_index_never_reaches_lowering() {
    let source = VIEW.replace(" and index < self.len", "");
    let (_directory, path) = fixture(&source);
    let output = command(&path).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).is_empty(),
        "{output:?}"
    );
}

#[test]
fn an_element_write_lowers_to_a_typed_store_and_a_mut_borrow_of_one_is_refused_by_name() {
    let writer = "#unrestricted\nByte is in 0 to 255\nBytes has\n    len in 0 to 1000\n    data mut [Byte; len]\n    put(self mut, index in 0 to 1000 and index < self.len, value Byte):\n        self.data.[index] :: value\n        return\n";
    let mlir = emitted(writer);
    assert!(mlir.contains("llvm.store %narrow"), "{mlir}");
    assert!(mlir.contains("-> (i64, i64)"), "{mlir}");
    // Negative control: no bound, no store.
    let unbounded = writer.replace(" and index < self.len", "");
    let (_directory, path) = fixture(&unbounded);
    let output = command(&path).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    // A `mut` borrow of an element would hand out an address: refused, naming the construct.
    let borrow = "#unrestricted\nByte is in 0 to 255\nBytes has\n    len in 0 to 1000\n    data mut [Byte; len]\n    slot(self mut, index in 0 to 1000 and index < self.len) mut Byte:\n        return mut self.data.[index]\n";
    let (_directory, path) = fixture(borrow);
    let output = command(&path).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
}

const PRINT_ONE: &str = "\
use core.io write
use core.span Extent, Vacant, Live, unmap
use core.slice Byte, SliceView

show(live ref Live(Byte) and live.start = 0):
    bytes: live.view
    write(bytes)
    return

#entry
main:
    mapped: Extent.map(4096)
    when mapped is Mapped(token) then
        vacant: token.claim(Byte, 4)
        live: Live.empty_at(Byte, vacant)
        piece: vacant.fill(104 as Byte)
        live.absorb(piece)
        show(live)
        last: live.take_last(vacant)
        live.finish
        unmap(Byte, vacant)
        return
    return
";

fn core_program(main: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"probe\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), main).unwrap();
    directory
}

/// The whole path of the element-access decision on a real machine: pages from the OS, a span
/// claimed from them, a byte stored, the span lent as a view, and the view written to stdout.
#[test]
fn a_span_prints_its_bytes_through_its_view_natively() {
    let directory = core_program(PRINT_ONE);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let ran = Command::new(&program).output().unwrap();
    assert_eq!(ran.stdout, b"h");
    assert_eq!(ran.status.code(), Some(0));
    // Negative control: a program that never fills the span prints nothing and still ends cleanly.
    let empty = PRINT_ONE
        .replace(
            "        piece: vacant.fill(104 as Byte)\n        live.absorb(piece)\n",
            "",
        )
        .replace("        last: live.take_last(vacant)\n", "");
    let directory = core_program(&empty);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(Command::new(&program).output().unwrap().stdout, b"");
}

#[test]
fn comparisons_of_address_sized_numbers_preserve_unsigned_and_mixed_ranges() {
    let unsigned = "pick(a in 0 to 18446744073709551615, b in 0 to 18446744073709551615) in 0 to 1:\n    when a > b then\n        return 1\n    return 0\n";
    assert!(
        emitted(unsigned).contains("arith.cmpi ugt"),
        "{}",
        emitted(unsigned)
    );
    // Different register interpretations are extended exactly before comparing.
    let mixed = "pick(a in 0 to 18446744073709551615, b in -9223372036854775808 to 9223372036854775807) in 0 to 1:\n    when a > b then\n        return 1\n    return 0\n";
    let text = emitted(mixed);
    assert!(text.contains("arith.extui"), "{text}");
    assert!(text.contains("arith.extsi"), "{text}");
    assert!(text.contains("arith.cmpi sgt"), "{text}");
    assert!(text.contains(": i128"), "{text}");
}

#[test]
fn a_while_loop_carries_a_record_as_its_registers() {
    // `rest` is two registers carried through every pass; the loop pops five times.
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/record_loop.gin"
    ))
    .unwrap();
    assert_eq!(exit_code(&source), Some(5));
    // Control: a loop that never runs leaves the record as it was.
    let none = source.replace("Stack(n: 5, top: 2)", "Stack(n: 0, top: 2)");
    assert_eq!(exit_code(&none), Some(0));
}

const VECTOR_HELLO: &str = "\
use core.io write
use core.span Extent, Vacant, unmap
use core.vector Vector
use core.slice Byte, Length, SliceView
use core.ops Release

greet(storage Vacant(Byte) and storage.start = 0 and storage.length = 8) Vacant(Byte):
    v: Vector.new(Byte, storage)
    v.push(104 as Byte)
    v.push(105 as Byte)
    v.push(10 as Byte)
    write(v.view)
    return v.release

#entry
main:
    mapped: Extent.map(4096)
    when mapped is Mapped(token) then
        vacant: token.claim(Byte, 8)
        rest: greet(vacant)
        unmap(Byte, rest)
        return
    return
";

/// A `Vector` over real pages: three pushes, a view written to stdout, and a release that ends
/// every remaining item (a loop carrying the vector's eight registers) before the pages go back.
#[test]
fn a_vector_pushes_lends_a_view_and_releases_natively() {
    let directory = core_program(VECTOR_HELLO);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let ran = Command::new(&program).output().unwrap();
    assert_eq!(ran.stdout, b"hi\n");
    assert_eq!(ran.status.code(), Some(0));
}

#[test]
fn a_vector_push_past_its_capacity_and_a_leaked_vector_are_refused() {
    // Negative controls: capacity 2 holds two, the third is unproved; dropping the vector leaks.
    let over = VECTOR_HELLO.replace("storage.length = 8", "storage.length = 2");
    let directory = core_program(&over.replace("claim(Byte, 8)", "claim(Byte, 2)"));
    let output = exe_command(directory.path(), &directory.path().join("program"))
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    let leaked = VECTOR_HELLO.replace("    return v.release\n", "    return storage\n");
    let directory = core_program(&leaked);
    let output = exe_command(directory.path(), &directory.path().join("program"))
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
}

/// Elements are read through the view: the byte at index 1 is the exit status.
#[test]
fn an_element_of_a_vector_is_read_through_its_view_natively() {
    let source = "\
use core.span Extent, Vacant, unmap
use core.vector Vector
use core.slice Byte, Length, SliceView
use core.ops Release

second(storage Vacant(Byte) and storage.start = 0 and storage.length = 4) in 0 to 255:
    v: Vector.new(Byte, storage)
    v.push(104 as Byte)
    v.push(105 as Byte)
    code in 0 to 255: look(v)
    rest: v.release
    unmap(Byte, rest)
    return code

look(v ref Vector(Byte) and v.length > 1) in 0 to 255:
    seen: v.view
    got: seen.borrow(1 as Length)
    return got.as_num

#entry
main() in 0 to 255:
    mapped: Extent.map(4096)
    when mapped is Mapped(token) then
        vacant: token.claim(Byte, 4)
        return second(vacant)
    return 0
";
    let directory = core_program(source);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(Command::new(&program).status().unwrap().code(), Some(105));
    // Negative control: reading index 1 of a one-element vector is unproved.
    let one = source.replace("    v.push(105 as Byte)\n", "");
    let directory = core_program(&one);
    let output = exe_command(directory.path(), &directory.path().join("program"))
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
}

const LIST_HELLO: &str = "\
use core.io write
use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.slice Byte, SliceView
use core.ops Release

#entry
main:
    attempt: List.with_capacity(Byte, GlobalAllocator, 4, GlobalAllocator())
    when attempt is Allocated(items) then
        items.push(104 as Byte)
        items.push(105 as Byte)
        items.push(10 as Byte)
        write(items.view)
        items.release
        return
    Failed(failure) := attempt
    failure.release
    return
";

/// A `List` whose pages come from the global allocator's `reserve` (guards on the size, an
/// `Extent` mapped and claimed), pushed, viewed, printed and released.
#[test]
fn a_list_over_the_global_allocator_pushes_prints_and_releases_natively() {
    let directory = core_program(LIST_HELLO);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let ran = Command::new(&program).output().unwrap();
    assert_eq!(ran.stdout, b"hi\n");
    assert_eq!(ran.status.code(), Some(0));
    // Negative controls: a fourth push into capacity 3 is unproved; a list never released leaks.
    let over = LIST_HELLO
        .replace("4, GlobalAllocator()", "3, GlobalAllocator()")
        .replace(
            "        items.push(10 as Byte)\n",
            "        items.push(10 as Byte)\n        items.push(33 as Byte)\n",
        );
    let directory = core_program(&over);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let leaked = LIST_HELLO.replace(
        "        write(items.view)\n        items.release\n        return\n",
        "        write(items.view)\n        return\n",
    );
    let directory = core_program(&leaked);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
}

const BOX_PEEK: &str = "\
use core.alloc Box, BoxFailure, new_box, GlobalAllocator
use core.slice Byte
use core.ops Release

peek(b ref Box(Byte, GlobalAllocator)) in 0 to 255:
    r: b.borrow
    return r.as_num

#entry
main() in 0 to 255:
    made: new_box(Byte, 77 as Byte)
    when made is Boxed(held) then
        code in 0 to 255: peek(held)
        held.release
        return code
    Failed(failure) := made
    failure.release
    return 0
";

/// A `Box` over the global allocator: a one-slot vector, read through its view, then released.
#[test]
fn a_box_holds_a_value_lends_it_and_releases_natively() {
    let directory = core_program(BOX_PEEK);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(Command::new(&program).status().unwrap().code(), Some(77));
    // Negative controls: a box not released leaks; a box cannot be released while it is lent.
    let leaked = BOX_PEEK.replace(
        "        held.release\n        return code\n",
        "        return code\n",
    );
    let directory = core_program(&leaked);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let lent = BOX_PEEK.replace(
        "        code in 0 to 255: peek(held)\n",
        "        r: held.borrow\n        held.release\n        code in 0 to 255: r.as_num\n",
    );
    let directory = core_program(&lent);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
}

const STRING_PRINT: &str = "\
use core.io write
use core.string String
use core.collection ListFailure
use core.alloc GlobalAllocator
use core.slice Byte
use core.ops Release

second(text ref String(GlobalAllocator) and text.length > 1) in 0 to 255:
    b: text.byte(1)
    return b.as_num

#entry
main() in 0 to 255:
    attempt: String.with_capacity(GlobalAllocator, 4, GlobalAllocator())
    when attempt is Made(held) then
        held.push(104 as Byte)
        held.push(105 as Byte)
        held.push(10 as Byte)
        write(held.as_bytes)
        code in 0 to 255: second(held)
        held.release
        return code
    Failed(failure) := attempt
    failure.release
    return 0
";

/// A `String`: capacity from the global allocator, bytes pushed, printed through its view, one
/// byte read back by index, released.
#[test]
fn a_string_is_built_printed_and_read_natively() {
    let directory = core_program(STRING_PRINT);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let ran = Command::new(&program).output().unwrap();
    assert_eq!(ran.stdout, b"hi\n");
    assert_eq!(ran.status.code(), Some(105));
    // Negative controls: reading past the length, pushing past the capacity.
    let past = STRING_PRINT.replace("text.length > 1", "text.length > 0");
    let directory = core_program(&past);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let over = STRING_PRINT.replace(
        "with_capacity(GlobalAllocator, 4,",
        "with_capacity(GlobalAllocator, 2,",
    );
    let directory = core_program(&over);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
}

/// A range `for` with an invariant carries a record through its passes, and the invariant gives
/// the total effect: five pops of five leave nothing, four pops leave one.
#[test]
fn a_for_loop_carries_a_record_and_its_invariant_gives_the_total_effect() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/record_for_loop.gin"
    ))
    .unwrap();
    assert_eq!(exit_code(&source), Some(0));
    let four = source.replace("for i in 0 to 4 where", "for i in 0 to 3 where");
    assert_eq!(exit_code(&four), Some(1));
    // Negative control: a wrong invariant is refused at entry.
    let wrong = source.replace("rest.count + i = 5", "rest.count + i = 4");
    let (_directory, path) = fixture(&wrong);
    assert!(!command(&path).output().unwrap().status.success());
}

/// `String.copy` and `String.concat` loop over a string's bytes: the loop invariant
/// (`self.length = start + index`) gives the new string's length, and a string's `length` is the
/// same number each time it is read.
#[test]
fn a_string_is_copied_and_concatenated_natively() {
    for (fixture, expected) in [
        ("string_copy.gin", &b"hi\n"[..]),
        ("string_concat.gin", &b"hi\nhi\n"[..]),
    ] {
        let source = std::fs::read_to_string(format!(
            "{}/tests/fixtures/{fixture}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let directory = core_program(&source);
        let program = directory.path().join("program");
        let output = exe_command(directory.path(), &program).output().unwrap();
        assert!(output.status.success(), "{fixture}: {output:?}");
        assert_eq!(
            Command::new(&program).output().unwrap().stdout,
            expected,
            "{fixture}"
        );
    }
    // Negative control: a copy that is never released leaks.
    let source = std::fs::read_to_string(format!(
        "{}/tests/fixtures/string_copy.gin",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let leaked = source.replace(
        "        emit(held)\n        held.release\n        return\n",
        "        emit(held)\n        return\n",
    );
    assert_ne!(leaked, source);
    let directory = core_program(&leaked);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
}

const HELLO_TEXT: &str = "\
use core.io write

#entry
main:
    write('Hello, world!')
    write('Hello, world!')
    write('héllo')
    return
";

/// A text literal used where a read-only view of bytes is wanted: one private read-only global per
/// distinct content, the length exact, and the program prints it.
#[test]
fn a_text_literal_is_a_read_only_global_viewed_as_bytes_and_printed() {
    let directory = core_program(HELLO_TEXT);
    let mlir = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "mlir"])
        .arg(directory.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&mlir.stdout);
    // Two distinct contents used three times: one private constant global each (the library's own
    // literals, such as the sign `-`, add theirs), the second 6 bytes. The repeated one is shared.
    assert!(
        text.contains("llvm.mlir.global private constant @gin.text."),
        "{text}"
    );
    assert_eq!(text.matches("!llvm.array<13 x i8>").count(), 1, "{text}");
    assert_eq!(text.matches("!llvm.array<6 x i8>").count(), 1, "{text}");
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let ran = Command::new(&program).output().unwrap();
    assert_eq!(ran.stdout, "Hello, world!Hello, world!héllo".as_bytes());
}

const PRINT_NUMBERS: &str = "\
use core.io write_int, print_newline, print_byte

#entry
main:
    write_int(0)
    print_newline
    write_int(7)
    print_newline
    write_int(-5)
    print_newline
    write_int(1234567890)
    print_newline
    write_int(9223372036854775807)
    print_newline
    write_int(-9223372036854775807 - 1)
    print_newline
    print_byte(65)
    print_newline
    return
";

/// Numbers in decimal through a nineteen-slot buffer and a sub-view from the first digit on: zero,
/// a digit, a negative, ten digits, the largest and the smallest 64-bit values.
#[test]
fn numbers_are_printed_in_decimal_through_a_sub_view_of_their_digits() {
    let directory = core_program(PRINT_NUMBERS);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let ran = Command::new(&program).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "0\n7\n-5\n1234567890\n9223372036854775807\n-9223372036854775808\nA\n"
    );
}

#[test]
fn a_literal_view_has_its_exact_utf8_length_and_is_never_writable() {
    let measure = "\
use core.slice Byte, Length, SliceView

six(bytes ref SliceView(Byte)) in 0 to 1:
    when bytes.len = 6 then
        return 1
    return 0

#entry
main() in 0 to 1:
    return six('héllo')
";
    let directory = core_program(measure);
    let program = directory.path().join("program");
    assert!(
        exe_command(directory.path(), &program)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(Command::new(&program).status().unwrap().code(), Some(1));
    // Negative controls: a `mut` view of a literal is refused (read-only data) ...
    let writable = measure.replace("SliceView", "SliceMutView");
    let directory = core_program(&writable);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
    // ... and an index past the literal's length is unproved: the empty literal has no first byte.
    let first = "\
use core.slice Byte, Length, SliceView

head(bytes ref SliceView(Byte) and bytes.len > 0) in 0 to 255:
    b: bytes.borrow(0 as Length)
    return b.as_num

#entry
main() in 0 to 255:
    return head('')
";
    let directory = core_program(first);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let ok = first.replace("head('')", "head('A')");
    let directory = core_program(&ok);
    let program = directory.path().join("program");
    assert!(
        exe_command(directory.path(), &program)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(Command::new(&program).status().unwrap().code(), Some(65));
}

const SUB_VIEW: &str = "\
use core.io write
use core.span Extent, Vacant, unmap
use core.vector Vector
use core.slice Byte, Length, SliceView
use core.ops Release

tail(storage Vacant(Byte) and storage.start = 0 and storage.length = 8) Vacant(Byte):
    v: Vector.new(Byte, storage)
    v.push(104 as Byte)
    v.push(105 as Byte)
    v.push(10 as Byte)
    write(v.view_from(2, 1))
    return v.release

#entry
main:
    mapped: Extent.map(4096)
    when mapped is Mapped(token) then
        vacant: token.claim(Byte, 8)
        rest: tail(vacant)
        unmap(Byte, rest)
        return
    return
";

/// A part of a view that does not start at the block: the address moves on by `from` elements and
/// the length is the part's own.
#[test]
fn a_sub_view_starts_inside_its_vector_and_ends_inside_it() {
    let directory = core_program(SUB_VIEW);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(Command::new(&program).output().unwrap().stdout, b"i\n");
    // Negative controls: a part that ends past the vector is unproved; a part is a loan on its
    // vector, so the vector cannot be pushed to while the part lives.
    let past = SUB_VIEW.replace("v.view_from(2, 1)", "v.view_from(3, 1)");
    let directory = core_program(&past);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let conflict = SUB_VIEW.replace(
        "    write(v.view_from(2, 1))\n",
        "    part: v.view_from(2, 1)\n    v.push(33 as Byte)\n    write(part)\n",
    );
    let directory = core_program(&conflict);
    assert!(
        !exe_command(directory.path(), &directory.path().join("p"))
            .output()
            .unwrap()
            .status
            .success()
    );
}

/// Build `main` against the core library and run it with `input` on standard input.
fn run_with_input(main: &str, inputs: &[&str]) -> Vec<(String, i32)> {
    use std::io::Write;
    use std::process::Stdio;
    let directory = core_program(main);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    inputs
        .iter()
        .map(|input| {
            let mut child = Command::new(&program)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            // A program that stops reading early closes the pipe: the write may fail.
            let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
            let done = child.wait_with_output().unwrap();
            (
                String::from_utf8_lossy(&done.stdout).into_owned(),
                done.status.code().unwrap_or(-1),
            )
        })
        .collect()
}

const READ_NUMBERS: &str = "\
use core.io read_int, write_int, print_newline, ReadResult
use core.primitive SignedBigInt

#entry
main in 0 to 2:
    left_read: read_int
    when left_read is Unread(error) then
        when error is Overflow then
            return 2
        return 1
    Value(left_value) := left_read
    left in SignedBigInt: left_value
    right_read: read_int
    when right_read is Unread(error) then
        when error is Overflow then
            return 2
        return 1
    Value(right_value) := right_read
    right in SignedBigInt: right_value
    write_int(left)
    print_newline
    write_int(right)
    print_newline
    return 0
";

/// Whole numbers are read from standard input: blanks and newlines between them, a sign, the
/// limits kept; a number that is not there is `NoNumber` (status 1), and one past 64 bits is
/// `Overflow` (status 2), with nothing printed.
#[test]
fn whole_numbers_are_read_from_standard_input() {
    let ran = run_with_input(
        READ_NUMBERS,
        &[
            "3 8\n",
            "  -3\n\n 42",
            "9223372036854775807 -9223372036854775808\n",
            "99999999999999999999 1\n",
            "9223372036854775808 1\n",
            "1 -9223372036854775809\n",
            "",
            "abc",
            "7x 5",
            "-",
            "5",
        ],
    );
    let expected = [
        ("3\n8\n", 0),
        ("-3\n42\n", 0),
        ("9223372036854775807\n-9223372036854775808\n", 0),
        ("", 2),
        ("", 2),
        ("", 2),
        ("", 1),
        ("", 1),
        // The byte that ends a number is consumed with it (`x`), so the next number is `5`.
        ("7\n5\n", 0),
        ("", 1),
        ("", 1),
    ];
    for ((got, status), (want, code)) in ran.iter().zip(expected) {
        assert_eq!((got.as_str(), *status), (want, code), "{ran:?}");
    }
}

const NEGATE_AFTER_GUARD: &str = "\
use core.io write_int, print_newline
use core.primitive SignedBigInt

magnitude(value in -9223372036854775808 to 9223372036854775807) in 0 to 9223372036854775807:
    when value < -9223372036854775807 then
        return 9223372036854775807
    when value < 0 then
        return 0 - value
    return value

#entry
main:
    write_int(magnitude(-5) as SignedBigInt)
    print_newline
    write_int(magnitude(7) as SignedBigInt)
    print_newline
    write_int(magnitude(-9223372036854775807) as SignedBigInt)
    print_newline
    return
";

/// A number negated after the guards that keep it away from the smallest value: the operand is
/// typed by what the guards tell, so the negation fits a register, and it runs natively. Without
/// the guard the checker refuses the negation.
#[test]
fn a_number_negated_after_its_guard_is_typed_by_the_guard() {
    let ran = run_with_input(NEGATE_AFTER_GUARD, &[""]);
    assert_eq!(ran[0], ("5\n7\n9223372036854775807\n".to_owned(), 0));
    let unguarded = NEGATE_AFTER_GUARD.replace(
        "    when value < -9223372036854775807 then\n        return 9223372036854775807\n",
        "",
    );
    let directory = core_program(&unguarded);
    let output = exe_command(directory.path(), &directory.path().join("program"))
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("source-check-"),
        "{output:?}"
    );
}

/// Standard input comes back byte for byte (`echo`), and a byte at a time (`read_character`:
/// the code of each byte, 256 at the end).
#[test]
fn standard_input_is_echoed_and_read_a_byte_at_a_time() {
    let echo = "use core.io echo\n\n#entry\nmain:\n    echo\n    return\n";
    let ran = run_with_input(echo, &["", "abc\n", "héllo\n\n", "a\0b"]);
    assert_eq!(ran[0], (String::new(), 0));
    assert_eq!(ran[1], ("abc\n".to_owned(), 0));
    assert_eq!(ran[2], ("héllo\n\n".to_owned(), 0));
    assert_eq!(ran[3], ("a\0b".to_owned(), 0));
    let codes = "\
use core.io read_character, write_int, print_newline

#entry
main:
    first := read_character
    second := read_character
    third := read_character
    write_int(first)
    print_newline
    write_int(second)
    print_newline
    write_int(third)
    print_newline
    return
";
    let ran = run_with_input(codes, &["A\u{e9}", "", "\u{ff}"]);
    // `é` is the two bytes 195 169; the end of input is 256 (and stays 256).
    assert_eq!(ran[0].0, "65\n195\n169\n");
    assert_eq!(ran[1].0, "256\n256\n256\n");
    assert_eq!(ran[2].0, "195\n191\n256\n");
}

const SLOTTED_ARRAY: &str = "#unrestricted\nByte is in 0 to 255\nByte has\n    as_num(self) in 0 to 255: self\nBag has\n    storage [Byte; 8]\n\n    new() Self: return Self(storage: [0 as Byte; 8])\n\n    get(self ref, index in 0 to 7) Byte: return self.storage.[index]\n\n    put(self mut, index in 0 to 7, value Byte):\n        self.storage.[index] :: value\n        return\n\n    release(self):\n        Bag(storage) := self\n        for item in storage do\n            ()\n        return\n\n#entry\nstart() in 0 to 255:\n    b: Bag.new\n    b.put(3, 42 as Byte)\n    v: b.get(3)\n    b.release\n    return v.as_num\n";

const SLOTTED_RECORD: &str = "#unrestricted\nByte is in 0 to 255\nByte has\n    as_num(self) in 0 to 255: self\n#unrestricted\nWide has\n    a0 in 0 to 1000\n    a1 in 0 to 1000\n    a2 in 0 to 1000\n    a3 in 0 to 1000\n    a4 in 0 to 1000\n    a5 in 0 to 1000\n    a6 in 0 to 1000\n    a7 in 0 to 1000\n    a8 in 0 to 1000\n    a9 in 0 to 1000\n    a10 in 0 to 1000\n    a11 in 0 to 1000\n    a12 in 0 to 1000\n    a13 in 0 to 1000\n    a14 in 0 to 1000\n    a15 in 0 to 1000\n    a16 in 0 to 1000\n    a17 in 0 to 1000\n    a18 in 0 to 1000\n    a19 in 0 to 1000\n\nmake(x in 0 to 100) Wide: return Wide(a0: x + 0, a1: x + 1, a2: x + 2, a3: x + 3, a4: x + 4, a5: x + 5, a6: x + 6, a7: x + 7, a8: x + 8, a9: x + 9, a10: x + 10, a11: x + 11, a12: x + 12, a13: x + 13, a14: x + 14, a15: x + 15, a16: x + 16, a17: x + 17, a18: x + 18, a19: x + 19)\n\ntotal(w ref Wide) in 0 to 2000: return w.a0 + w.a19\n\nbump(w mut Wide):\n    w.a5 :: 7\n    return\n\n#entry\nmain in 0 to 255:\n    w: make(3)\n    bump(w)\n    return (total(w) + w.a5 + w.a6) % 256\n";

fn run_status(source: &str) -> Option<i32> {
    let directory = package(&[("main.gin", source)]);
    let program = directory.path().join("program");
    let output = exe_command(directory.path(), &program).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    Command::new(&program).status().unwrap().code()
}

fn refused_to_build(source: &str) {
    let directory = package(&[("main.gin", source)]);
    let output = exe_command(directory.path(), &directory.path().join("program"))
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
}

/// An owned array of a known length lives in a stack slot and is passed by its address: made in
/// `new` (whose slot ends with it, so it is copied into a slot of the caller's), written in place
/// through a `mut` receiver, read back. Controls: an array made in a loop body (each pass would
/// write over the last) and a sum that would carry one out are refused; an index past the length is
/// unproved by the checker.
#[test]
fn an_owned_array_is_held_in_a_stack_slot_and_passed_by_its_address() {
    assert_eq!(run_status(SLOTTED_ARRAY), Some(42));
    let looped = SLOTTED_ARRAY.replace(
        "#entry\n",
        "churn(n in 0 to 5):\n    for i in 1 to n do\n        c: Bag.new\n        c.release\n    return\n\n#entry\n",
    ).replace("    b.release\n    return v.as_num\n", "    churn(v.as_num % 6)\n    b.release\n    return v.as_num\n");
    refused_to_build(&looped);
    let past = SLOTTED_ARRAY.replace(
        "get(self ref, index in 0 to 7)",
        "get(self ref, index in 0 to 8)",
    );
    let directory = package(&[("main.gin", past.as_str())]);
    let output = exe_command(directory.path(), &directory.path().join("p"))
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("source-check-unproved"),
        "{output:?}"
    );
}

/// A record of twenty numbers, wider than the sixteen registers: held in a slot, passed by address,
/// a field written through `mut` (the callee's new record is copied back into the caller's slot),
/// and every value read back exactly (3 + 22 + 7 + 9). A record of a few numbers still travels in
/// registers.
#[test]
fn a_record_wider_than_the_registers_round_trips_through_a_slot() {
    assert_eq!(run_status(SLOTTED_RECORD), Some(41));
    let directory = package(&[("main.gin", SLOTTED_RECORD)]);
    let mlir = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "mlir"])
        .arg(directory.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&mlir.stdout);
    assert!(text.contains("total\"(%arg0: i64) -> i64"), "{text}");
    assert!(text.contains("llvm.alloca"), "{text}");
    let small = "#unrestricted\nPair has\n    a in 0 to 9\n    b in 0 to 9\n\nsum(p Pair) in 0 to 18: return p.a + p.b\n\n#entry\nstart() in 0 to 255: return sum(Pair(a: 4, b: 5))\n";
    let directory = package(&[("main.gin", small)]);
    let mlir = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "mlir"])
        .arg(directory.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&mlir.stdout);
    assert!(
        text.contains("sum\"(%arg0_0: i64, %arg0_1: i64)") && !text.contains("llvm.alloca"),
        "{text}"
    );
    assert_eq!(run_status(small), Some(9));
}

const SLOT_OWNER: &str = "#unrestricted\nByte is in 0 to 255\nByte has\n    as_num(self) in 0 to 255: self\nBag has\n    storage [Byte; 8]\n\n    new() Self: return Self(storage: [0 as Byte; 8])\n\n    get(self ref, index in 0 to 7) Byte: return self.storage.[index]\n\n    put(self mut, index in 0 to 7, value Byte):\n        self.storage.[index] :: value\n        return\n\n    view(self ref) ref [Byte; 8]: return ref self.storage\n\n    address(self mut) @Byte: return #unchecked(self.storage as @Byte)\n\n    release(self):\n        Bag(storage) := self\n        for item in storage do\n            ()\n        return\n\n";

/// The address of a slotted value is a loan on its owner: a view of the owned array cannot outlive
/// it (used after the owner ends, or returned out of the function that owns it), and the owner
/// cannot be moved while the view is held. (A raw pointer from `#unchecked(.. as @Byte)` is the
/// library's trust, by design: no loan.)
#[test]
fn a_view_of_a_slotted_array_cannot_outlive_or_move_its_owner() {
    let cases = [
        (
            "#entry\nmain in 0 to 255:\n    b: Bag.new\n    v: b.view\n    b.release\n    x: v.[2]\n    return 0\n",
            "borrow-conflict",
        ),
        (
            "leak() ref [Byte; 8]:\n    b: Bag.new\n    v: b.view\n    b.release\n    return v\n\n#entry\nmain in 0 to 255:\n    return 0\n",
            "borrow-conflict",
        ),
        (
            "consume(b Bag):\n    b.release\n    return\n\n#entry\nmain in 0 to 255:\n    b: Bag.new\n    v: b.view\n    consume(b)\n    x: v.[2]\n    return 0\n",
            "source-check-",
        ),
        (
            "#entry\nmain in 0 to 255:\n    b: Bag.new\n    b.release\n    q: b.address\n    return 0\n",
            "use-after-move",
        ),
    ];
    for (tail, code) in cases {
        let source = format!("{SLOT_OWNER}{tail}");
        let (_directory, path) = fixture(&source);
        let output = command(&path).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{tail}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(code),
            "{tail}: {output:?}"
        );
    }
}

/// `write` writes every byte of a view (`write_all`: again from where a short write stopped): a
/// view of 300000 bytes built with `push` reaches a slow reader whole, in order, and an empty view
/// writes nothing.
#[test]
fn write_all_writes_a_large_view_whole() {
    let source = "\
use core.io write, write_all
use core.slice Byte
use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.ops Release

#entry
main:
    attempt: List.with_capacity(Byte, GlobalAllocator, 300000, GlobalAllocator())
    when attempt is Allocated(items) then
        for index in 0 to 299999 where items.length = index do
            items.push((65 + index % 26) as Byte)
        write_all(items.view)
        write(items.view_from(0, 0))
        items.release
        return
    Failed(failure) := attempt
    failure.release
    return
";
    let ran = run_with_input(source, &[""]);
    assert_eq!(ran[0].1, 0);
    let want: String = (0..300000)
        .map(|i| (b'A' + (i % 26) as u8) as char)
        .collect();
    assert_eq!(ran[0].0, want);
}

/// `List.borrow(index)` lends one element for as long as the list is unchanged.
#[test]
fn a_list_lends_one_of_its_elements() {
    let source = "\
use core.io write_int
use core.slice Byte
use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.ops Release

show(list ref List(Byte, GlobalAllocator) and list.length = 3):
    second: list.borrow(1)
    write_int(second.as_num)
    return

#entry
main:
    attempt: List.with_capacity(Byte, GlobalAllocator, 4, GlobalAllocator())
    when attempt is Allocated(items) then
        items.push(7 as Byte)
        items.push(8 as Byte)
        items.push(9 as Byte)
        show(items)
        items.release
        return
    Failed(failure) := attempt
    failure.release
    return
";
    let ran = run_with_input(source, &[""]);
    assert_eq!(ran[0], ("8".to_owned(), 0));
}

/// The terminal's `Output` buffers escape sequences, numbers and a queued view, and `flush` writes
/// them out whole: clear screen, cursor to row 5 column 4 (numbers of up to five digits, no leading
/// zeros), and text. Controls: a view past the buffer is `BufferFull` and queues nothing.
#[test]
#[ignore = "Output.queue is declined by the recorder: it counts 5 domain conversions where the checker counts 3"]
fn the_terminal_output_buffers_and_flushes_escape_sequences() {
    let directory = tempfile::tempdir().unwrap();
    let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
    for name in ["core", "terminal"] {
        std::os::unix::fs::symlink(
            modules.join(name).canonicalize().unwrap(),
            directory.path().join(name),
        )
        .unwrap();
    }
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"probe\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\nterminal: \"./terminal\"\n",
    )
    .unwrap();
    let main = "\
use terminal.output Output, succeeded

#entry
main in 0 to 9:
    out: Output.new(1)
    first: out.clear
    when succeeded(first) = 0 then
        out.release
        return 1
    second: out.move_to(3, 4)
    when succeeded(second) = 0 then
        out.release
        return 2
    third: out.queue('hi')
    when succeeded(third) = 0 then
        out.release
        return 3
    shown: out.flush
    when succeeded(shown) = 0 then
        out.release
        return 4
    out.release
    return 0
";
    std::fs::write(directory.path().join("main.gin"), main).unwrap();
    let program = directory.path().join("program");
    let built = exe_command(directory.path(), &program).output().unwrap();
    assert!(built.status.success(), "{built:?}");
    let ran = Command::new(&program).output().unwrap();
    assert_eq!(ran.stdout, b"\x1b[2J\x1b[5;4Hhi");
}
