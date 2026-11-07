//! Control flow lowers to MLIR and the lowered function computes what the source says for every
//! input: guards that return and fall-through conditionals become compares and selects, and
//! `while` becomes `scf.while`.
//!
//! `when n < 2 then return 0` followed by the rest of the body is a choice between the two
//! values. The arms are computed together, which is sound
//! only for pure arms; the first test pins the shape, the second runs it.
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const SOURCE: &str = "\
clamp(n in 0 to 9) in 0 to 7:
    when n < 2 then return 0
    when n > 8 then return 7
    return n - 2
";

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

#[test]
fn the_guards_are_compares_and_selects_with_the_first_outermost() {
    let (_directory, path) = fixture(SOURCE);
    let text = emit(&path);
    assert!(text.contains("arith.cmpi slt"), "{text}");
    assert!(text.contains("arith.cmpi sgt"), "{text}");
    assert_eq!(text.matches("arith.select").count(), 2, "{text}");
    assert!(!text.contains("cf.cond_br"), "{text}");
    // The last select is the first guard: it chooses between 0 and the later guards' result.
    let last = text.rfind("arith.select").unwrap();
    assert!(text[last..].contains("%const1"), "{text}");
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

#[test]
fn the_lowered_function_agrees_with_the_source_for_every_input() {
    run_every_input(SOURCE, "clamp", |n| {
        if n < 2 {
            0
        } else if n > 8 {
            7
        } else {
            n - 2
        }
    });
}

#[test]
fn an_or_returns_when_either_comparison_holds() {
    let source = "\
edge(n in 0 to 9) in 0 to 5:
    when n < 2 or n > 7 then return 0
    return n - 2
";
    run_every_input(source, "edge", |n| {
        if !(2..=7).contains(&n) { 0 } else { n - 2 }
    });
}

#[test]
fn an_and_returns_only_when_every_comparison_holds() {
    let source = "\
middle(n in 0 to 9) in 0 to 9:
    when n > 2 and n < 6 then return n - 3
    return 9
";
    run_every_input(source, "middle", |n| if n > 2 && n < 6 { n - 3 } else { 9 });
}

#[test]
fn a_chain_of_joined_assignments_runs_in_order() {
    let source = "\
chain(n in 0 to 9) in 0 to 9:
    m in 0 to 9: 0
    when n < 3 then m :: 2
    when n > 6 then m :: 6
    return m
";
    run_every_input(source, "chain", |n| {
        let mut m = 0;
        if n < 3 {
            m = 2;
        }
        if n > 6 {
            m = 6;
        }
        m
    });
}

#[test]
fn both_branches_of_a_join_compute_from_the_inputs() {
    let source = "\
spread(n in 0 to 8) in 0 to 9:
    m in 0 to 9: 0
    when n < 5 then
        m :: n + 1
    else
        m :: n
    return m
";
    run_every_input(source, "spread", |n| if n < 5 { n + 1 } else { n });
}

const WHILE_PIPELINE: [&str; 4] = [
    "--convert-scf-to-cf",
    "--convert-arith-to-llvm",
    "--convert-cf-to-llvm",
    "--convert-func-to-llvm",
];

#[test]
fn a_while_loop_emits_scf_while() {
    let source = "\
count(n in 0 to 9) in 0 to 10:
    i in 0 to 10: 0
    while i < n do
        i :: i + 1
    return i
";
    let (_directory, path) = fixture(source);
    let text = emit(&path);
    assert!(text.contains("scf.while"), "{text}");
    assert!(text.contains("scf.condition"), "{text}");
    assert!(text.contains("scf.yield"), "{text}");
}

#[test]
fn a_counting_while_loop_runs_n_times() {
    let source = "\
count(n in 0 to 9) in 0 to 10:
    i in 0 to 10: 0
    while i < n do
        i :: i + 1
    return i
";
    run_every_input_with(source, "count", &WHILE_PIPELINE, |n| n);
}

/// Two locals carried, with a conditional inside the body.
#[test]
fn a_while_loop_may_carry_several_locals_and_join_inside() {
    let source = "\
drain(n in 0 to 9) in 0 to 10:
    i in 0 to 10: 0
    j in 0 to 10: 10
    while i < n do
        i :: i + 1
        when j > 0 then
            j :: j - 1
    return j
";
    run_every_input_with(source, "drain", &WHILE_PIPELINE, |n| 10 - n);
}

#[test]
fn a_while_loop_may_sit_in_another() {
    let source = "\
nested(n in 0 to 9) in 0 to 10:
    i in 0 to 10: 0
    while i < n do
        k in 0 to 10: 0
        while k < i do
            k :: k + 1
        i :: i + 1
    return i
";
    run_every_input_with(source, "nested", &WHILE_PIPELINE, |n| n);
}

/// A loop that never runs (its condition cannot hold) is not emitted, and the function is just
/// the entry value.
#[test]
fn a_while_loop_that_cannot_start_is_not_emitted() {
    let source = "\
never(n in 0 to 9) in 0 to 10:
    i in 0 to 10: 3
    while i > 10 do
        i :: i + 1
    return i
";
    let (_directory, path) = fixture(source);
    let text = emit(&path);
    assert!(!text.contains("scf.while"), "{text}");
}

/// `for k in 0 to n` is inclusive: it runs `n + 1` times, here counting up to a cap.
#[test]
fn an_inclusive_range_for_runs_end_plus_one_times() {
    let source = "\
tally(n in 0 to 9) in 0 to 100:
    c in 0 to 100: 0
    for k in 0 to n do
        when c < 100 then
            c :: c + 1
    return c
";
    run_every_input_with(source, "tally", &WHILE_PIPELINE, |n| n + 1);
}

/// `scf.for` takes an exclusive bound, so the loop runs to `end + 1`. A domain reaching the
/// largest 64-bit number would wrap there, so such a loop is refused, not lowered wrongly; one
/// number below it lowers.
#[test]
fn a_range_for_whose_end_can_be_the_largest_number_is_refused() {
    let source = |upper: &str| {
        format!(
            "tally(n in 0 to {upper}) in 0 to 100:\n    c in 0 to 100: 0\n    for k in 0 to n do\n        when c < 100 then\n            c :: c + 1\n    return c\n"
        )
    };
    let run = |upper: &str| {
        let (_directory, path) = fixture(&source(upper));
        Command::new(env!("CARGO_BIN_EXE_ginc"))
            .arg("--emit")
            .arg("mlir")
            .arg(&path)
            .output()
            .unwrap()
    };
    let refused = run("9223372036854775807");
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("unsupported construct"),
        "{refused:?}"
    );
    let lowered = run("9223372036854775806");
    assert!(lowered.status.success(), "{lowered:?}");
    assert!(String::from_utf8_lossy(&lowered.stdout).contains("scf.for"));
}

/// A one-armed case tests a tag; the rest of the body is every other member.
#[test]
fn a_one_armed_case_dispatches_on_a_tag() {
    let source = "\
#unrestricted
Mode is Quiet or Loud or Silent

pick(n in 0 to 9) Mode:
    when n < 3 then return Mode.Quiet
    when n < 6 then return Mode.Loud
    return Mode.Silent

level(n in 0 to 9) in 0 to 9:
    when pick(n) is Loud then return 5
    when pick(n) is Silent then return 9
    return 1
";
    run_every_input(source, "level", |n| {
        if n < 3 {
            1
        } else if n < 6 {
            5
        } else {
            9
        }
    });
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

/// A `return` inside a `while`: the loop carries whether it has returned and what it returned,
/// stops at the first one, and the function's result is that value if it ran.
#[test]
fn a_return_inside_a_while_loop_ends_the_function() {
    let source = "\
first(n in 0 to 9) in 0 to 20:
    i in 0 to 10: 0
    while i < 10 do
        when i * i > n then return i
        i :: i + 1
    return 20
";
    run_every_input_with(source, "first", &WHILE_PIPELINE, |n| {
        (0..10).find(|i| i * i > n).unwrap_or(20)
    });
}

/// The first `return` wins when several hold in one pass.
#[test]
fn the_first_return_in_a_pass_wins() {
    let source = "\
pick(n in 0 to 9) in 0 to 99:
    i in 0 to 10: 0
    while i < 10 do
        when i > 2 then return 11
        when i > 0 then return 22
        i :: i + 1
    return 99
";
    run_every_input_with(source, "pick", &WHILE_PIPELINE, |_| 22);
}

/// A loop that never reaches its `return` falls through to the code after it.
#[test]
fn a_loop_whose_return_never_holds_falls_through() {
    let source = "\
scan(n in 0 to 9) in 0 to 50:
    i in 0 to 10: 0
    while i < n do
        when i = 15 then return 1
        i :: i + 1
    return i + 30
";
    run_every_input_with(source, "scan", &WHILE_PIPELINE, |n| n + 30);
}

/// A one-armed case as the exit of a loop: the tag of a computed value decides.
#[test]
fn a_one_armed_case_can_end_a_loop() {
    let source = "\
#unrestricted
Mode is Quiet or Loud

classify(k in 0 to 9) Mode:
    when k > 6 then return Mode.Loud
    return Mode.Quiet

first_loud(n in 0 to 9) in 0 to 10:
    i in 0 to 10: n
    while i < 10 do
        when classify(i) is Loud then return i
        i :: i + 1
    return 10
";
    run_every_input_with(source, "first_loud", &WHILE_PIPELINE, |n| n.max(7));
}

// `not` and `is` in conditions, run: the lowered code must agree with the source.

#[test]
fn a_negated_conjunction_guard_runs() {
    let source = "\
edge(n in 0 to 9) in 0 to 5:
    when not (n >= 2 and n <= 7) then return 0
    return n - 2
";
    run_every_input_with(source, "edge", &WHILE_PIPELINE, |n| {
        if (2..=7).contains(&n) { n - 2 } else { 0 }
    });
}

/// `not (n = 4)` is a not-equal, which a recorded comparison expresses as an equal with the
/// branches swapped.
#[test]
fn a_not_equal_guard_runs() {
    let source = "\
other(n in 0 to 9) in 1 to 2:
    when not (n = 4) then return 1
    return 2
";
    run_every_input_with(
        source,
        "other",
        &WHILE_PIPELINE,
        |n| if n != 4 { 1 } else { 2 },
    );
}

#[test]
fn is_on_a_number_runs_as_equality() {
    let source = "\
swap(n in 0 to 9) in 0 to 9:
    when n is 3 then return 7
    return n
";
    run_every_input_with(
        source,
        "swap",
        &WHILE_PIPELINE,
        |n| if n == 3 { 7 } else { n },
    );
}

#[test]
fn a_not_equal_loop_condition_runs() {
    let source = "\
count_to(n in 0 to 9) in 0 to 10:
    i in 0 to 10: 0
    while not (i = 10) do
        when i < 10 then
            i :: i + 1
    return i
";
    run_every_input_with(source, "count_to", &WHILE_PIPELINE, |_| 10);
}

#[test]
fn a_negated_loop_condition_runs() {
    let source = "\
upto(n in 0 to 9) in 0 to 10:
    i in 0 to 10: 0
    while not i >= n do
        i :: i + 1
    return i
";
    run_every_input_with(source, "upto", &WHILE_PIPELINE, |n| n);
}

const MODES: &str = "\
#unrestricted
Mode is Quiet or Loud or Silent

pick(n in 0 to 9) Mode:
    when n < 3 then return Mode.Quiet
    when n < 6 then return Mode.Loud
    return Mode.Silent
";

#[test]
fn a_value_conditional_on_a_variant_test_runs() {
    let source = format!(
        "{MODES}\nvolume(n in 0 to 9) in 0 to 9: return when pick(n) is Loud then 5 else 1\n"
    );
    run_every_input_with(&source, "volume", &WHILE_PIPELINE, |n| {
        if (3..6).contains(&n) { 5 } else { 1 }
    });
}

#[test]
fn a_negated_variant_conditional_runs() {
    let source = format!(
        "{MODES}\nquiet(n in 0 to 9) in 0 to 9: return when not pick(n) is Loud then 1 else 5\n"
    );
    run_every_input_with(&source, "quiet", &WHILE_PIPELINE, |n| {
        if (3..6).contains(&n) { 5 } else { 1 }
    });
}

// Domain membership, run.

#[test]
fn a_value_conditional_on_membership_runs() {
    let source = "\
inside(n in 0 to 9) in 0 to 1: return when n is in 3 to 6 then 1 else 0
";
    run_every_input_with(source, "inside", &WHILE_PIPELINE, |n| {
        i32::from((3..=6).contains(&n))
    });
}

#[test]
fn a_membership_guard_runs() {
    let source = "\
swap(n in 0 to 9) in 0 to 9:
    when n is in 3 to 6 then return 7
    return n
";
    run_every_input_with(source, "swap", &WHILE_PIPELINE, |n| {
        if (3..=6).contains(&n) { 7 } else { n }
    });
}

#[test]
fn a_negated_membership_runs() {
    let source = "\
outside(n in 0 to 9) in 0 to 9:
    when not n is in 3 to 6 then return 0
    return n
";
    run_every_input_with(source, "outside", &WHILE_PIPELINE, |n| {
        if (3..=6).contains(&n) { n } else { 0 }
    });
}

#[test]
fn a_membership_join_runs() {
    let source = "\
pick(n in 0 to 9) in 0 to 9:
    m in 0 to 9: 0
    when n is in 3 to 6 then m :: n
    return m
";
    run_every_input_with(source, "pick", &WHILE_PIPELINE, |n| {
        if (3..=6).contains(&n) { n } else { 0 }
    });
}
