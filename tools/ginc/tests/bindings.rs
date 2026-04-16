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
