mod common;
use common::{compile_exe, compile_exe_checked, require_success};
use ginc::cli::Profile;
use std::process::Command;
use test_fixtures::TempPackage;

#[test]
fn direct_generic_union_returns_round_trip_across_modules() {
    let package = TempPackage::new("direct_union_returns");
    package.write_flask("direct_store");
    package.write(
        "store/store.gin",
        &format!("{}#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has\n    finish(self):\n        return\nBlock(x) has\n    value x\n    capacity Count(x)\n    finish(self):\n        return\nError is Empty or Full\nStore has\n    reserve(self mut, x, value x, count Count(x)) Block(x) or Error\nConcrete has Store\n    reserve(self mut, x, value x, count Count(x)) Block(x) or Error:\n        n is in Count(x): count\n        when n = 0 then\n            return Empty\n        when n = 255 then\n            return Full\n        return Block(value, capacity: count)\n    finish(self):\n        return\nforward(x, value x, count Count(x)) Block(x) or Error:\n    store: Concrete()\n    result: store.reserve(x, value, count)\n    store.finish\n    return result\ninspect(block Block(Word)) Word:\n    Block(value, capacity) := block\n    n is in Count(Word): capacity\n    return when n = 4 then value else 254\ninspect_error(error Error) Word: return when error is Empty then 10 else 20\n", test_fixtures::preludes::WORD_PRELUDE),
    );
    let input = package.write(
        "main.gin",
        r#"use direct_store.store Word, Count, forward, inspect, inspect_error
run(value Word, count Count(Word)) Word:
    result: forward(Word, value, count)
    return when result is
        Block(block) then inspect(block)
        Error(error) then inspect_error(error)
#entry
main:
    success is in Word: run(7, 4 as Count(Word))
    empty is in Word: run(7, 0 as Count(Word))
    full is in Word: run(7, 255 as Count(Word))
    return when success = 7 and empty = 10 and full = 20 then 0 else 1
"#,
    );
    for profile in [Profile::Debug, Profile::Release] {
        let output = package.join(format!("program-{profile:?}"));
        compile_exe_checked(input.clone(), &output, profile);
        assert!(
            Command::new(output).status().unwrap().success(),
            "{profile:?}"
        );
    }
}

#[test]
fn generic_record_type_sum_payloads_round_trip_at_runtime() {
    let package = TempPackage::new("generic_type_sum");
    let input = package.write(
        "main.gin",
        include_str!("../../../crates/test-fixtures/fixtures/type_sum_generic_payloads.gin"),
    );
    for profile in [Profile::Debug, Profile::Release] {
        let output = package.join(format!("program-{profile:?}"));
        compile_exe_checked(input.clone(), &output, profile);
        assert!(
            Command::new(output).status().unwrap().success(),
            "{profile:?}"
        );
    }
}

#[test]
fn named_success_and_nested_failures_round_trip_at_runtime() {
    let dependency = TempPackage::new("type_sum_values");
    dependency.write_flask("outcomes");
    dependency.write(
        "number.gin",
        r#"
Word is in 0 to 255
Outcome is Word or Failure(Empty or Full)
Outcome has
    finish(self):
        return
OtherOutcome is Word or Failure(Small or Large)
OtherOutcome has
    finish(self):
        return

Word has
    finish(self):
        return
    #infix('/')
    divide(lhs Self, rhs Self) Outcome:
        left is in 0 to 255: lhs
        right is in 0 to 255: rhs
        return when right = 0 then Failure(Empty) else (left / right) as Self

    copy(value ref Self) Self:
        bits is in 0 to 255: value
        return bits as Self

choose(value Word) Outcome:
    bits is in 0 to 255: value
    return when bits = 0 then Failure(Empty) else when bits = 255 then Failure(Full) else bits as Word
"#,
    );
    let package = TempPackage::new("type_sum");
    package.write_flask_with_deps(
        "client",
        &format!(
            r#"outcomes: "../{}"
"#,
            dependency.path().file_name().unwrap().to_string_lossy(),
        ),
    );
    package.write("main.gin", r#"
use outcomes Word, Outcome, choose
take_after_copy(copy Word, value Word) Word:
    discarded is in 0 to 255: copy
    return value
recover(value Outcome) Word: return when value is Failure then 0 else take_after_copy(Word.copy(value), value)
divide_result(left Word, right Word) Word: return recover(left / right)
inspect(value Outcome) Word:
    return when value is
        Failure(reason) then when reason is Empty then 1 else 2
        Word(number) then number
check(value Word, expected Word) Word:
    actual is in 0 to 255: value
    wanted is in 0 to 255: expected
    return when actual = wanted then 0 else 1
#entry
main:
    number Word: 42
    a is in 0 to 255: check(recover(choose(Word.copy(number))), 42)
    b is in 0 to 255: check(recover(choose(0)), 0)
    c is in 0 to 255: check(recover(choose(255)), 0)
    d is in 0 to 255: check(inspect(choose(0)), 1)
    e is in 0 to 255: check(inspect(choose(255)), 2)
    f is in 0 to 255: check(inspect(choose(99)), 99)
    g is in 0 to 255: check(number, 42)
    h is in 0 to 255: check(divide_result(42, 1), 42)
    i is in 0 to 255: check(divide_result(42, 0), 0)
    return a + b + c + d + e + f + g + h + i
"#);
    let output = package.join("type_sum");
    let result = compile_exe(
        package.join("main.gin").parent().unwrap().to_path_buf(),
        &output.clone(),
        Profile::Debug,
    );
    require_success(&result, Profile::Debug);
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    assert!(Command::new(output).status().unwrap().success());
}
