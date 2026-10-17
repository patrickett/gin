mod common;

use common::*;

#[test]
fn test_integer_literal() {
    cra("integer_literal", "#entry\nmain: 42\n")
        .assert_compiled()
        .assert_exit_code(42);
}

#[test]
fn test_zero_literal() {
    cra("zero_literal", "#entry\nmain: 0\n")
        .assert_compiled()
        .assert_success();
}

#[test]
fn test_negative_integer() {
    cra("negative_integer", "#entry\nmain: -1\n")
        .assert_compiled()
        .assert_exit_code(255);
}

#[test]
fn test_large_integer() {
    // 9999999999 truncated to u8 → 255.
    cra("large_integer", "#entry\nmain: 9999999999\n")
        .assert_compiled()
        .assert_exit_code(255);
}

#[test]
fn test_string_literal() {
    cra(
        "string_literal",
        "#entry\nmain:\n    text [in 0 to 255; 5]: 'hello'\n    return #size(text)\n",
    )
    .assert_compiled()
    .assert_exit_code(5);
}
