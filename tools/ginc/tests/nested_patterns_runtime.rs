//! `Done(Full(value))`: a case arm that names a variant of the payload union, built and run
//! natively for every combination of the outer and inner variant.
use std::path::{Path, PathBuf};
use std::process::Command;

fn build(source: &str) -> Result<(tempfile::TempDir, PathBuf), String> {
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"nested\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\n",
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
Slot is Empty or Full(Count)

#unrestricted
Count is in 0 to 9

#unrestricted
Step is Done(Slot) or Pending

show(step Step):
    code in 0 to 9: when step is
        Pending then 0
        Done(Empty) then 1
        Done(Full(value)) then 2
    print_int(code)
    return

#entry
main in 0 to 1:
    show(Step.Pending)
    show(Step.Done(Slot.Empty))
    show(Step.Done(Slot.Full(4 as Count)))
    return 0
";

#[test]
fn every_combination_takes_its_own_arm() {
    let (_directory, program) = build(PROGRAM).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "0\n1\n2\n");
    assert_eq!(done.status.code(), Some(0));
}

#[test]
fn swapping_the_inner_arms_swaps_the_output() {
    // Negative control: the inner variant, not just the outer, picks the arm.
    let swapped = PROGRAM
        .replace("Done(Empty) then 1", "Done(Empty) then 5")
        .replace("Done(Full(value)) then 2", "Done(Full(value)) then 6");
    let (_directory, program) = build(&swapped).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "0\n5\n6\n");
}

#[test]
fn a_labeled_nested_pattern_runs() {
    let labeled = PROGRAM
        .replace(
            "Step is Done(Slot) or Pending",
            "Step is Done(tag Count, slot Slot) or Pending",
        )
        .replace("Done(Empty) then 1", "Done(Empty: slot, _: tag) then 1")
        .replace(
            "Done(Full(value)) then 2",
            "Done(t: tag, Full(value): slot) then 2",
        )
        .replace(
            "Step.Done(Slot.Empty)",
            "Step.Done(tag: 1 as Count, slot: Slot.Empty)",
        )
        .replace(
            "Step.Done(Slot.Full(4 as Count))",
            "Step.Done(tag: 2 as Count, slot: Slot.Full(4 as Count))",
        );
    let (_directory, program) = build(&labeled).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "0\n1\n2\n");
}

#[test]
fn a_guarded_nested_arm_falls_through_to_the_unguarded_one() {
    let guarded = PROGRAM.replace(
        "        Done(Full(value)) then 2\n",
        "        Done(Full(value)) and value > 3 then 7\n        Done(Full(value)) then 2\n",
    )
    .replace("    show(Step.Done(Slot.Full(4 as Count)))\n", "    show(Step.Done(Slot.Full(4 as Count)))\n    show(Step.Done(Slot.Full(1 as Count)))\n");
    let (_directory, program) = build(&guarded).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "0\n1\n7\n2\n");
}

#[test]
fn flat_guarded_arms_over_a_sum_run_and_each_guard_decides() {
    let source = "\
use core.io print_int

#unrestricted
Count is in 0 to 9

#unrestricted
Step is Done(Count) or Pending

show(step Step, n in 0 to 9):
    code in 0 to 9: when step is
        Pending then 0
        Done(c) and n > 6 then 8
        Done(c) and n > 3 then 7
        Done(c) then 1
    print_int(code)
    return

#entry
main in 0 to 1:
    show(Step.Done(4 as Count), 9)
    show(Step.Done(4 as Count), 5)
    show(Step.Done(1 as Count), 1)
    show(Step.Pending, 9)
    return 0
";
    let (_directory, program) = build(source).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "8\n7\n1\n0\n");
}

const TRANSITIONS: &str = "\
use core.io print_int

#unrestricted
Counter has
    n in 0 to 100

bump(c mut Counter and c.n < 100) Grown where (c.n :: c.n + 1) or Kept where (c.n :: c.n):
    grown in 0 to 1: when c.n < 50 then 1 else 0
    c.n :: c.n + grown
    return when grown = 1 then Grown else Kept

report(start in 0 to 99):
    c: Counter(n: start)
    shown in 0 to 100: when bump(c) is
        Grown then c.n
        Kept then c.n
    print_int(shown)
    return

#entry
main in 0 to 1:
    report(3)
    report(70)
    return 0
";

#[test]
fn variant_transitions_run_natively() {
    let (_directory, program) = build(TRANSITIONS).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "4\n70\n");
    // Negative controls: the arm learns only its own variant's value, so a range that holds for
    // `Grown` alone is refused; and a body whose `Grown` exit breaks its transition is refused.
    let tight = TRANSITIONS.replace(
        "    shown in 0 to 100: when bump(c) is\n        Grown then c.n\n        Kept then c.n",
        "    shown in 4 to 4: when bump(c) is\n        Grown then c.n\n        Kept then c.n",
    );
    assert!(build(&tight).is_err());
    let lying = TRANSITIONS.replace("c.n :: c.n + grown", "c.n :: c.n + grown + grown");
    assert!(build(&lying).is_err());
}

#[test]
fn a_unit_case_with_an_effect_arm_and_a_returning_arm_runs() {
    let source = "\
use core.io print_int
use core.slice Byte
use core.ops CopyResult

Step is Done or Pending

Step has
    finish(self):
        when self is
            Done then print_int(1)
            Pending then return
        return

show(b ref Byte):
    r CopyResult(Byte): b.copy
    r.release
    print_int(2)
    return

#entry
main in 0 to 1:
    a Step: Step.Done
    a.finish
    p Step: Step.Pending
    p.finish
    show(7 as Byte)
    return 0
";
    let (_directory, program) = build(source).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    // `Pending` prints nothing: only the arm taken runs its effect.
    assert_eq!(String::from_utf8_lossy(&done.stdout), "1\n2\n");
}

#[test]
fn arms_mixing_a_number_and_a_domain_value_give_a_number() {
    let source = "\
use core.io print_int

#unrestricted
Count is in 0 to 9

#unrestricted
Step is Done(Count) or Pending or Twice(Count)

show(step Step, n in 0 to 9):
    code in 0 to 9: when step is
        Pending then 0
        Done(c) and n > 5 then 9
        Done(c) then c
        Twice(c) then c
    print_int(code)
    return

#entry
main in 0 to 1:
    show(Step.Done(4 as Count), 1)
    show(Step.Done(4 as Count), 7)
    show(Step.Pending, 1)
    show(Step.Twice(7 as Count), 1)
    return 0
";
    let (_directory, program) = build(source).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "4\n9\n0\n7\n");
}

#[test]
fn exits_that_mutate_differently_and_return_different_variants_run() {
    // The natural shape: a returning guard that mutates, and a fall-through exit that does not.
    let natural = TRANSITIONS.replace(
        "    grown in 0 to 1: when c.n < 50 then 1 else 0\n    c.n :: c.n + grown\n    return when grown = 1 then Grown else Kept",
        "    when c.n < 50 then\n        c.n :: c.n + 1\n        return Grown\n    return Kept",
    );
    assert_ne!(natural, TRANSITIONS);
    let (_directory, program) = build(&natural).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "4\n70\n");
}

#[test]
fn a_case_with_more_than_two_effectful_arms_runs_only_the_arm_taken() {
    let source = "\
use core.io print_int

#unrestricted
Count is in 0 to 9

Step is Done(Count) or Pending or Twice(Count) or Thrice

Step has
    finish(self):
        return when self is
            Done(c) then print_int(1)
            Pending then ()
            Twice(c) then print_int(2)
            Thrice then print_int(3)

#entry
main in 0 to 1:
    a Step: Step.Done(4 as Count)
    a.finish
    b Step: Step.Pending
    b.finish
    t Step: Step.Twice(4 as Count)
    t.finish
    h Step: Step.Thrice
    h.finish
    return 0
";
    let (_directory, program) = build(source).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "1\n2\n3\n");
}

#[test]
fn checked_division_and_remainder_run_natively_with_every_failure() {
    // `-1` is read at run time (end of input is 256), so nothing is folded while compiling.
    let source = r#"use core.io print_int, read_character
use core.primitive SignedBigInt, IntegerResult

div(a SignedBigInt, b SignedBigInt):
    return when a.divide(b) is
        Value(v) then print_int(v)
        Flaw(_) then print_int(-99 as SignedBigInt)

rem(a SignedBigInt, b SignedBigInt):
    return when a.modulo(b) is
        Value(v) then print_int(v)
        Flaw(_) then print_int(-99 as SignedBigInt)

#entry
main in 0 to 1:
    c in 0 to 256: read_character
    m1 in -257 to -1: c - 257
    div(7 as SignedBigInt, 2 as SignedBigInt)
    div(7 as SignedBigInt, 0 as SignedBigInt)
    div(-5 as SignedBigInt, m1 as SignedBigInt)
    div(-9223372036854775808 as SignedBigInt, m1 as SignedBigInt)
    div(7 as SignedBigInt, -3 as SignedBigInt)
    rem(7 as SignedBigInt, 2 as SignedBigInt)
    rem(7 as SignedBigInt, 0 as SignedBigInt)
    rem(-9223372036854775808 as SignedBigInt, m1 as SignedBigInt)
    rem(-7 as SignedBigInt, 3 as SignedBigInt)
    return 0
"#;
    let (_directory, program) = build(source).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&done.stdout),
        "3\n-99\n5\n-99\n-2\n1\n-99\n-99\n-1\n"
    );
}

#[test]
fn a_pattern_two_levels_deep_runs_natively() {
    let source = "\
use core.io print_int

#unrestricted
Code is in 0 to 9

#unrestricted
Inner is Low or High(Code)

#unrestricted
Slot is Empty or Full(Inner)

#unrestricted
Step is Done(Slot) or Pending

show(step Step):
    code in 0 to 9: when step is
        Pending then 0
        Done(Empty) then 1
        Done(Full(Low)) then 2
        Done(Full(High(n))) then n
    print_int(code)
    return

#entry
main in 0 to 1:
    show(Step.Pending)
    show(Step.Done(Slot.Empty))
    show(Step.Done(Slot.Full(Inner.Low)))
    show(Step.Done(Slot.Full(Inner.High(7 as Code))))
    return 0
";
    let (_directory, program) = build(source).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "0\n1\n2\n7\n");
    // Negative control: the deepest variant, not only the outer two, picks the arm.
    let swapped = source.replace("Done(Full(Low)) then 2", "Done(Full(Low)) then 5");
    let (_directory, program) = build(&swapped).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "0\n1\n5\n7\n");
}

// ---- audit additions (2026-11-05) ----

const OUTCOME: &str = "\
use core.io print_int

#unrestricted
Code is in 0 to 9

#unrestricted
AllocError is OutOfMemory or Fragmented

#unrestricted
Outcome is Value(Code) or Flaw(AllocError) or Other(AllocError)

show(r Outcome, n in 0 to 9):
    code in 0 to 9: when r is
        Value(v) then v
        Flaw(OutOfMemory) then 4
        Flaw(Fragmented) then 2
        Other(_) then 3
    print_int(code)
    return

#entry
main in 0 to 1:
    show(Outcome.Value(6 as Code), 9)
    show(Outcome.Flaw(AllocError.OutOfMemory), 9)
    show(Outcome.Flaw(AllocError.Fragmented), 9)
    show(Outcome.Other(AllocError.OutOfMemory), 9)
    show(Outcome.Other(AllocError.Fragmented), 9)
    return 0
";

#[test]
fn the_outer_binder_and_each_inner_variant_pick_their_own_arm() {
    let (_directory, program) = build(OUTCOME).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    // `Value(v)` hands back the payload itself; `Flaw` and `Other` hold the same inner union and
    // must not be confused with each other.
    assert_eq!(String::from_utf8_lossy(&done.stdout), "6\n4\n2\n3\n3\n");
    // Negative control: a guarded nested arm that does not apply changes nothing, and the
    // outer variant still decides (`Other(OutOfMemory)` never reaches a `Flaw` arm).
    let swapped = OUTCOME.replace("Other(_) then 3", "Other(_) then 8");
    let (_directory, program) = build(&swapped).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "6\n4\n2\n8\n8\n");
}

#[test]
fn a_guarded_nested_arm_runs_when_directly_before_its_fallback() {
    let adjacent = OUTCOME.replace(
        "        Flaw(OutOfMemory) then 4\n",
        "        Flaw(OutOfMemory) and n > 5 then 9\n        Flaw(OutOfMemory) then 4\n",
    );
    assert_ne!(adjacent, OUTCOME);
    let (_directory, program) = build(&adjacent).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    // n is 9 at every call, so the guard holds for `Flaw(OutOfMemory)` only.
    assert_eq!(String::from_utf8_lossy(&done.stdout), "6\n9\n2\n3\n3\n");
    // Negative control: with a guard that never holds, the fallback arm answers.
    let never = adjacent.replace("n > 5", "n > 9");
    let (_directory, program) = build(&never).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "6\n4\n2\n3\n3\n");
}

#[test]
fn a_guarded_nested_arm_separated_from_its_fallback_is_refused() {
    // Decided 2026-11-05 (B2): a guarded arm must sit directly before the unguarded arm of its
    // member, so the checker refuses this order (Duplicate) instead of the recorder declining it
    // later. This used to pin the recorder's "domain conversion" message.
    let apart = OUTCOME.replace(
        "        Flaw(OutOfMemory) then 4\n        Flaw(Fragmented) then 2\n        Other(_) then 3\n",
        "        Flaw(OutOfMemory) and n > 5 then 9\n        Other(_) then 3\n        Flaw(OutOfMemory) then 4\n        Flaw(Fragmented) then 2\n",
    );
    assert_ne!(apart, OUTCOME);
    let message = build(&apart).expect_err("the checker refuses this order");
    assert!(message.contains("source-check-duplicate"), "{message}");
}

#[test]
fn guarded_arms_out_of_order_are_refused() {
    // Decided 2026-11-05 (B2): the checker now refuses both orders below (a guarded nested arm
    // after the unguarded arm of its member, which can never run, and a flat guarded arm that is
    // not directly before its fallback). They used to be accepted and then declined by the recorder.
    let late = OUTCOME.replace(
        "        Flaw(OutOfMemory) then 4\n",
        "        Flaw(OutOfMemory) then 4\n        Flaw(OutOfMemory) and n > 5 then 9\n",
    );
    let flat = "use core.io print_int\n\n#unrestricted\nCount is in 0 to 9\n\n#unrestricted\nStep is Done(Count) or Pending\n\nshow(step Step, n in 0 to 9):\n    code in 0 to 9: when step is\n        Done(c) and n > 6 then 8\n        Pending then 0\n        Done(c) then 1\n    print_int(code)\n    return\n\n#entry\nmain in 0 to 1:\n    show(Step.Done(4 as Count), 9)\n    show(Step.Pending, 9)\n    return 0\n";
    for source in [late.as_str(), flat] {
        let message = build(source).expect_err("the checker refuses this order");
        assert!(message.contains("source-check-duplicate"), "{message}");
    }
    // Negative control: the same flat program with the guard directly before its fallback runs.
    let fixed = flat.replace(
        "        Done(c) and n > 6 then 8\n        Pending then 0\n        Done(c) then 1\n",
        "        Pending then 0\n        Done(c) and n > 6 then 8\n        Done(c) then 1\n",
    );
    let (_directory, program) = build(&fixed).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "8\n0\n");
}

fn build_files(files: &[(&str, &str)], main: &str) -> Result<(tempfile::TempDir, PathBuf), String> {
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"nested\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\n",
    )
    .unwrap();
    for (path, text) in files {
        let path = directory.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    std::fs::write(directory.path().join("main.gin"), main).unwrap();
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

fn stdout_of(program: &Path) -> String {
    String::from_utf8_lossy(&Command::new(program).output().unwrap().stdout).into_owned()
}

const OUTCOMES_LIBRARY: &str = "\
#unrestricted
Code is in 0 to 9

#unrestricted
AllocError is OutOfMemory or Fragmented

#unrestricted
Outcome is Value(Code) or Flaw(AllocError)

classify(n in 0 to 9) Outcome:
    when n < 3 then
        return Value(n as Code)
    when n < 6 then
        return Flaw(OutOfMemory)
    return Flaw(Fragmented)
";

const OUTCOMES_MAIN: &str = "\
use core.io print_int
use nested.outcomes Outcome, classify

rank(n in 0 to 9):
    code in 0 to 9: when classify(n) is
        Value(v) then v
        Flaw(OutOfMemory) then 7
        Flaw(Fragmented) then 8
    print_int(code)
    return

#entry
main in 0 to 1:
    rank(2)
    rank(4)
    rank(9)
    return 0
";

#[test]
fn a_nested_match_over_an_imported_union_runs() {
    let files = [("outcomes/outcomes.gin", OUTCOMES_LIBRARY)];
    let (_directory, program) =
        build_files(&files, OUTCOMES_MAIN).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stdout_of(&program), "2\n7\n8\n");
    // Negative control: leaving out an inner variant is refused by the checker as a missing arm
    // (`Type`), not by the build for some other reason.
    let missing = OUTCOMES_MAIN.replace("        Flaw(Fragmented) then 8\n", "");
    let message = build_files(&files, &missing).expect_err("a missing inner arm is refused");
    assert!(message.contains("source-check-type"), "{message}");
}

const TOKENS: &str = "\
use core.io print_int

Token has n in 0 to 3

Held is Some(Token) or None

Wrapped is Inside(Held) or Outside

read(token Token) in 0 to 3:
    Token(n) := token
    return n

take(w Wrapped) in 0 to 3: return when w is
    Outside then 0
    Inside(Some(token)) then read(token)
    Inside(None) then 1

#entry
main in 0 to 1:
    a Wrapped: Wrapped.Outside
    b Wrapped: Wrapped.Inside(Held.None)
    c Wrapped: Wrapped.Inside(Held.Some(Token(n: 3)))
    print_int(take(a))
    print_int(take(b))
    print_int(take(c))
    return 0
";

#[test]
fn a_nested_pattern_over_a_linear_payload_consumes_it_and_runs() {
    let (_directory, program) = build(TOKENS).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stdout_of(&program), "0\n1\n3\n");
    // Negative control: an arm that drops the linear payload without consuming it is refused
    // for that reason, so the success above rests on the consumption being checked.
    let leaked = TOKENS.replace(
        "Inside(Some(token)) then read(token)",
        "Inside(Some(token)) then 2",
    );
    let message = build(&leaked).expect_err("a leaked linear payload is refused");
    assert!(message.contains("source-check-unconsumed"), "{message}");
}

#[test]
fn a_block_arm_that_destructures_its_binder_runs() {
    // G3 (2026-11-05): `Token(n) := token` in a multi-statement arm of a value `when` records
    // the binder as a payload read, like the destructure at the top of a function. The arm's
    // value is its `return` (the `when` is the function's returned value); the linear payload is consumed exactly once, by the destructure.
    let nested = TOKENS.replace(
        "    Inside(Some(token)) then read(token)\n",
        "    Inside(Some(token)) then\n        Token(n) := token\n        return n\n",
    );
    assert_ne!(nested, TOKENS);
    let (_directory, program) = build(&nested).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stdout_of(&program), "0\n1\n3\n");
    // Flat pattern, same destructure.
    let flat = "use core.io print_int\n\nToken has n in 0 to 3\n\nHeld is Some(Token) or None\n\ntake(h Held) in 0 to 3: return when h is\n    None then 1\n    Some(token) then\n        Token(n) := token\n        return n\n\n#entry\nmain in 0 to 1:\n    a Held: Held.None\n    b Held: Held.Some(Token(n: 2))\n    print_int(take(a))\n    print_int(take(b))\n    return 0\n";
    let (_directory, program) = build(flat).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stdout_of(&program), "1\n2\n");
    // Negative controls: consuming the payload twice, or not at all, is refused by the checker.
    let twice = nested.replace(
        "        Token(n) := token\n        return n\n",
        "        Token(n) := token\n        Token(m) := token\n        return n\n",
    );
    let message = build(&twice).expect_err("a payload consumed twice is refused");
    assert!(message.contains("source-check-use-after-move"), "{message}");
    let dropped = nested.replace(
        "        Token(n) := token\n        return n\n",
        "        return 3\n",
    );
    let message = build(&dropped).expect_err("a dropped linear payload is refused");
    assert!(message.contains("source-check-unconsumed"), "{message}");
}

const CHECKED_LIBRARY: &str = "\
#unrestricted
Code is in 0 to 9

check(n in 0 to 9) Fine where (n < 5) or Failed(Code) where (n >= 5):
    when n < 5 then
        return Fine
    return Failed(n as Code)
";

const CHECKED_MAIN: &str = "\
use core.io print_int
use nested.checks check

rank(n in 0 to 9):
    code in 0 to 9: when check(n) is
        Fine then n
        Failed(c) then c
    print_int(code)
    return

#entry
main in 0 to 1:
    rank(3)
    rank(7)
    return 0
";

#[test]
fn an_imported_inline_result_with_claims_runs_and_each_arm_learns_its_own_claim() {
    let files = [("checks/checks.gin", CHECKED_LIBRARY)];
    let (_directory, program) = build_files(&files, CHECKED_MAIN).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stdout_of(&program), "3\n7\n");
    // Negative control 1: a claim the returned variant does not meet is refused in the library.
    let lying =
        CHECKED_LIBRARY.replace("Failed(Code) where (n >= 5)", "Failed(Code) where (n >= 7)");
    let message =
        build_files(&[("checks/checks.gin", &lying)], CHECKED_MAIN).expect_err("false claim");
    assert!(message.contains("source-check-unproved"), "{message}");
    // Negative control 2: the `Fine` arm knows `n < 5` only, so `n + 6` can reach 10 and the 0 to 9 result
    // is refused (`n + 5` would fit, since n < 5 is assumed).
    let tight = CHECKED_MAIN.replace("        Fine then n\n", "        Fine then n + 6\n");
    let message =
        build_files(&files, &tight).expect_err("the claim of the other variant is not assumed");
    assert!(message.contains("source-check-"), "{message}");
}

const MAKER: &str = "\
use core.io print_int

#unrestricted
Code is in 0 to 9

Maker has
    make(self, n in 0 to 9) Fine or Failed(Code)

#unrestricted
Plain has Maker
    k in 0 to 9

    make(self, n in 0 to 9) Fine or Failed(Code):
        when n < self.k then
            return Fine
        return Failed(n as Code)

rank(p Plain, n in 0 to 9):
    code in 0 to 9: when p.make(n) is
        Fine then 0
        Failed(c) then c
    print_int(code)
    return

#entry
main in 0 to 1:
    rank(Plain(k: 5), 2)
    rank(Plain(k: 5), 8)
    return 0
";

#[test]
fn a_provider_of_a_trait_with_an_inline_result_runs() {
    let (_directory, program) = build(MAKER).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stdout_of(&program), "0\n8\n");
    // Negative control: a provider whose result is not the trait's is refused as a type error.
    let wrong = MAKER.replace(
        "make(self, n in 0 to 9) Fine or Failed(Code):",
        "make(self, n in 0 to 9) Fine or Broken(Code):",
    );
    let message = build(&wrong).expect_err("a provider must repeat the trait's variants");
    assert!(message.contains("source-check-type"), "{message}");
}

const REGISTER: &str = "\
use core.io print_int

#unrestricted
Register is Low or High

flip(r Register) Register: return when r is
    Low then Register.High
    High then Register.Low

show(r Register):
    code in 0 to 1: when r is
        Low then 0
        High then 1
    print_int(code)
    return

twice(r Register):
    show(r)
    show(r)
    return

#entry
main in 0 to 1:
    twice(Register.High)
    show(flip(Register.High))
    return 0
";

#[test]
fn an_unrestricted_union_is_copied_and_a_linear_one_is_refused() {
    let (_directory, program) = build(REGISTER).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stdout_of(&program), "1\n1\n0\n");
    // Negative control: without `#unrestricted` the union is linear, and the second use of the
    // parameter `r` is a use after move. (A local bound from a known variant is NOT refused today:
    // see the ignored test `a_local_bound_from_a_known_variant_of_a_linear_union_is_linear` in
    // typecheck `union_audit`.)
    let linear = REGISTER.replace("#unrestricted\nRegister is", "Register is");
    let message = build(&linear).expect_err("a linear union cannot be used twice");
    assert!(message.contains("source-check-use-after-move"), "{message}");
}

const NAMED: &str = "\
use core.io print_int

#unrestricted
Code is in 0 to 9

#unrestricted
Result is Fine or Failed(code Code, detail Code) or Aborted(proposal in 1 to 16)

show(r Result):
    out in 0 to 16: when r is
        Fine then 0
        Failed(a, b) then b
        Aborted(p) then p
    print_int(out)
    return

labeled(r Result):
    out in 0 to 9: when r is
        Fine then 0
        Failed(x: code, y: detail) then x
        Aborted(p) then 9
    print_int(out)
    return

#entry
main in 0 to 1:
    show(Result.Fine)
    show(Result.Failed(code: 4 as Code, detail: 7 as Code))
    show(Result.Aborted(12))
    labeled(Result.Failed(code: 4 as Code, detail: 7 as Code))
    return 0
";

#[test]
fn named_payload_fields_and_a_range_typed_payload_run() {
    let (_directory, program) = build(NAMED).unwrap_or_else(|e| panic!("{e}"));
    // Positional binders read the fields in declaration order; the labeled form reads by name.
    assert_eq!(stdout_of(&program), "0\n7\n12\n4\n");
    // Negative control: a payload outside the declared range is refused as a type error.
    let outside = NAMED.replace("Result.Aborted(12)", "Result.Aborted(30)");
    let message = build(&outside).expect_err("30 is outside 1 to 16");
    assert!(message.contains("source-check-type"), "{message}");
    // Negative control: swapping the binders changes the answer (4 -> 7), so the labels pick fields.
    let swapped = NAMED.replace(
        "Failed(x: code, y: detail) then x",
        "Failed(x: code, y: detail) then y",
    );
    let (_directory, program) = build(&swapped).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stdout_of(&program), "0\n7\n12\n7\n");
}

// ---- tail form in a block arm (2026-11-05, surface-syntax decision 1) ----

const TAIL_ARM: &str = "\
use core.io print_int

#unrestricted
Count is in 0 to 9

#unrestricted
Step is Done(Count) or Pending

show(step Step):
    code in 0 to 9: when step is
        Pending then 0
        Done(c) then
            doubled in 0 to 9: 3
            @@
    print_int(code)
    return

pick(step Step) in 0 to 9:
    return when step is
        Pending then 0
        Done(c) then
            c

#entry
main in 0 to 1:
    show(Step.Done(4 as Count))
    show(Step.Pending)
    print_int(pick(Step.Done(6 as Count)))
    print_int(pick(Step.Pending))
    return 0
";

#[test]
fn a_block_arm_ends_in_its_value_without_return() {
    let source = TAIL_ARM.replace("@@", "doubled");
    let (_directory, program) = build(&source).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "3\n0\n6\n0\n");
    // Negative control: the last line is the value, so changing it changes the output.
    let other = TAIL_ARM.replace("@@", "c");
    let (_directory, program) = build(&other).unwrap_or_else(|e| panic!("{e}"));
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "4\n0\n6\n0\n");
}

#[test]
fn a_tail_value_outside_the_wanted_range_is_refused() {
    // Negative control: the tail line is checked against the `when`'s domain like any arm value.
    let source = TAIL_ARM.replace("@@", "c + 5");
    assert!(build(&source).is_err());
}

// ---- effects before the value of an arm block (2026-11-05) ----

const EFFECT_ARM: &str = "\
use core.io print_int

#unrestricted
Count is in 0 to 9

#unrestricted
Step is Done(Count) or Pending

pick(step Step) in 0 to 9:
    return when step is
        Pending then 0
        Done(c) then
            print_int(9)
@@

#entry
main in 0 to 1:
    print_int(pick(Step.Done(4 as Count)))
    print_int(pick(Step.Pending))
    return 0
";

#[test]
fn an_effect_before_the_value_runs_only_when_its_arm_is_taken() {
    for last in ["            3", "            return 3"] {
        let source = EFFECT_ARM.replace("@@", last);
        let (_directory, program) = build(&source).unwrap_or_else(|e| panic!("{e}"));
        let done = Command::new(&program).output().unwrap();
        // `Pending` prints only its own 0: the effect of the other arm did not run.
        assert_eq!(String::from_utf8_lossy(&done.stdout), "9\n3\n0\n", "{last}");
    }
}

#[test]
fn an_effect_beside_a_domain_valued_arm_is_declined_not_run_in_every_arm() {
    // Known gap (item 3): an arm that gives a domain value beside one that gives a number is
    // read as a number after the arms are recorded, so no branch can be made around the effect.
    // It was a miscompile (the effect ran in `Pending` too); it is now refused by lowering.
    let source = EFFECT_ARM.replace("@@", "            c");
    let error = build(&source).expect_err("declined");
    assert!(error.contains("unsupported construct"), "{error}");
}

#[test]
fn a_return_inside_a_value_when_arm_is_a_known_recorder_gap() {
    // Item 4 (2026-11-05), pinned: the checker accepts it, the recorder declines (it has no
    // branches, so a skipped effect after the exit cannot be expressed; fixture-matrix R7).
    let source = "\
use core.io print_int

#unrestricted
Count is in 0 to 9

#unrestricted
Step is Done(Count) or Pending

pick(step Step) in 0 to 9:
    x in 0 to 9: when step is
        Pending then return 5
        Done(c) then 3
    print_int(x)
    return x

#entry
main in 0 to 1:
    return 0
";
    let error = build(source).expect_err("declined");
    assert!(error.contains("recorder declined"), "{error}");
}
