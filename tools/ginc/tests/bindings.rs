mod common;

use common::*;

#[test]
fn test_let_binding() {
    compile_and_run_with_options(
        "x: 10\nmain: x + 5\n",
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
        "LogLevel is 'debug' or 'info' or 'warn' or 'error'

main:
    level LogLevel: 'debug'
    return 0
return
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
        "LogLevel is 'debug' or 'info' or 'warn' or 'error'

main:
    level LogLevel: 'debug'
    result: when level
        is 'debug': 1
        is 'info': 2
        is 'warn': 3
        is 'error': 4
        else 0
    return result
return
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
        "LogLevel is 'debug' or 'info' or 'warn' or 'error'

main:
    level LogLevel: 'info'
    result: when level
        is 'debug': 1
        is 'info': 2
        else 0
    return result
return
",
        Options {
            test_name: "const_union_when_else".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(2);
}

#[test]
fn test_const_union_pass_to_function() {
    compile_and_run_with_options(
        "LogLevel is 'debug' or 'info' or 'warn' or 'error'

main:
    level LogLevel: 'error'
    result: when level is 'debug' then 10 else 20
    return result
return
",
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
        "x: 10\ny: 20\nmain: x + y\n",
        Options {
            test_name: "multiple_bindings".into(),
            ..Default::default()
        },
    )
    .assert_compiled()
    .assert_exit_code(30);
}
