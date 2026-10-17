mod common;

use common::*;

#[test]
fn test_let_binding() {
    compile_and_run_with_options(
        "x: 10\n#entry\nmain: x + 5\n",
        Options {
            test_name: "let_binding".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(15);
}

#[test]
fn test_const_union_construction() {
    compile_and_run_with_options(
        "LogLevel is Debug or Info or Warn or Error

#entry
main:
    level LogLevel: Debug
    return 0
",
        Options {
            test_name: "const_union_construction".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(0);
}

#[test]
fn test_const_union_when_match() {
    compile_and_run_with_options(
        "LogLevel is Debug or Info or Warn or Error

#entry
main:
    level LogLevel: Debug
    result: when level is
        Debug then 1
        Info then 2
        Warn then 3
        Error then 4
    return result
",
        Options {
            test_name: "const_union_when_match".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(1);
}

#[test]
fn test_const_union_when_else() {
    compile_and_run_with_options(
        "LogLevel is Debug or Info or Warn or Error

#entry
main:
    level LogLevel: Warn
    result: when level is
        Debug then 1
        Info then 2
        else 0
    return result
",
        Options {
            test_name: "const_union_when_else".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(0);
}

#[test]
fn test_const_union_pass_to_function() {
    compile_and_run_with_options(
        "LogLevel is Debug or Info or Warn or Error\nlog_code(level LogLevel): return when level is Debug then 10 else 20\n\n#entry\nmain:\n    level LogLevel: Error\n    result: log_code(level)\n    return result\n",
        Options {
            test_name: "const_union_pass_to_fn".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(20);
}

#[test]
fn test_multiple_bindings() {
    compile_and_run_with_options(
        "x: 10\ny: 20\n#entry\nmain: x + y\n",
        Options {
            test_name: "multiple_bindings".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(30);
}

#[test]
fn expression_bodied_functions_return_inferred_union_values() {
    compile_and_run_with_options(
        r#"Word is in 0 to 255
Word.finish(self): return
Left has value Word
Right has value Word
Third has value Word
pair(flag Word): when flag = 0 then Left(value: 7) else Right(value: 3)
pick(flag Word): when flag = 2 then Third(value: 5) else pair(flag)
score(flag Word) Word: when pick(flag) is
    Left(left) then left.value
    Right(right) then right.value
    Third(third) then third.value
#entry
main:
    local(flag Word): when flag = 0 then 11 else 13
    first is in Word: score(0)
    second is in Word: score(1)
    third is in Word: score(2)
    return first + second + third + local(0) + local(1)
"#,
        Options {
            test_name: "expression_bodied_union_functions".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(39);
}
