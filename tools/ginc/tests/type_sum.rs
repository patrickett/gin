use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn named_success_and_nested_failures_round_trip_at_runtime() {
    let dependency = TempPackage::new("type_sum_values");
    dependency.write_flask("outcomes");
    dependency.write("number.gin", r#"
Word is in 0 to 255
Outcome is Word or Failure(Empty or Full)
OtherOutcome is Word or Failure(Small or Large)

Word has
    #infix('/')
    divide(lhs Self, rhs Self) Outcome:
        left is in 0 to 255: lhs
        right is in 0 to 255: rhs
    return when right = 0 is True then Failure(Empty) else (left / right) as Self

    copy(value ref Self) Self:
        bits is in 0 to 255: value
    return bits as Self

choose(value Word) Outcome:
    bits is in 0 to 255: value
return when bits = 0 is True then Failure(Empty) else when bits = 255 is True then Failure(Full) else bits as Word
"#);
    let package = TempPackage::new("type_sum");
    package.write_flask_with_deps(
        "client",
        &format!(
            r#"{{"outcomes":{{"path":"../{}"}}}}"#,
            dependency.path().file_name().unwrap().to_string_lossy(),
        ),
    );
    package.write("main.gin", r#"
use outcomes Word, Outcome, choose
take_after_copy(copy Word, value Word) Word:
    discarded is in 0 to 255: copy
return value
recover(value Outcome) Word: when value is Failure then 0 else take_after_copy(Word.copy(ref value), value)
divide_result(left Word, right Word) Word: recover(left / right)
inspect(value Outcome) Word:
return when value is
    Failure(reason) then when reason is Empty then 1 else 2
    Word(number) then number
check(value Word, expected Word) Word:
    actual is in 0 to 255: value
    wanted is in 0 to 255: expected
return when actual = wanted is True then 0 else 1
main:
    number Word: 42
    a is in 0 to 255: check(recover(choose(Word.copy(ref number))), 42)
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
    let mut args = Args {
        input: package.join("main.gin").parent().unwrap().to_path_buf(),
        output: Some(output.clone()),
        emit: Emit::Exe,
        profile: Profile::Debug,
        target: Some(host_target_triple()),
        no_cache: true,
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(result.is_success(), "{result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    assert!(Command::new(output).status().unwrap().success());
}
