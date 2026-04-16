mod common;
use common::*;

#[test]
fn test_addition() {
    cra("addition", "main: 1 + 2\n")
        .assert_compiled()
        .assert_exit_code(3);
}

#[test]
fn test_subtraction() {
    cra("subtraction", "main: 10 - 3\n")
        .assert_compiled()
        .assert_exit_code(7);
}

#[test]
fn test_multiplication() {
    cra("multiplication", "main: 4 * 5\n")
        .assert_compiled()
        .assert_exit_code(20);
}

#[test]
fn test_left_to_right_precedence() {
    // Gin evaluates left-to-right: 2 + 3 * 4 = (2 + 3) * 4 = 20.
    cra("precedence", "main: 2 + 3 * 4\n")
        .assert_compiled()
        .assert_exit_code(20);
}

#[test]
fn test_parenthesized_expression() {
    cra("parenthesized", "main: (2 + 3) * 4\n")
        .assert_compiled()
        .assert_exit_code(20);
}
