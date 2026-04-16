mod common;

use common::*;

#[test]
fn test_integer_literal() {
    cra("integer_literal", "main: 42\n")
        .assert_compiled()
        .assert_exit_code(42);
}

#[test]
fn test_zero_literal() {
    cra("zero_literal", "main: 0\n")
        .assert_compiled()
        .assert_success();
}

#[test]
fn test_negative_integer() {
    // -1 wraps to 255 as an unsigned byte exit code.
    cra("negative_integer", "main: -1\n")
        .assert_compiled()
        .assert_exit_code(255);
}

#[test]
fn test_large_integer() {
    // 9999999999 truncated to u8 → 255.
    cra("large_integer", "main: 9999999999\n")
        .assert_compiled()
        .assert_exit_code(255);
}

#[test]
fn test_string_literal() {
    // String literals compile; exit code is ABI-dependent.
    cra("string_literal", "main: 'hello'\n").assert_compiled();
}
