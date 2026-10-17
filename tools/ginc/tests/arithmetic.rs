mod common;
use common::*;

#[test]
fn test_addition() {
    cra("addition", "#entry\nmain: 1 + 2\n")
        .assert_compiled()
        .assert_exit_code(3);
}

#[test]
fn test_subtraction() {
    cra("subtraction", "#entry\nmain: 10 - 3\n")
        .assert_compiled()
        .assert_exit_code(7);
}

#[test]
fn test_multiplication() {
    cra("multiplication", "#entry\nmain: 4 * 5\n")
        .assert_compiled()
        .assert_exit_code(20);
}

#[test]
fn test_left_to_right_precedence() {
    // Gin evaluates left-to-right: 2 + 3 * 4 = (2 + 3) * 4 = 20.
    cra("precedence", "#entry\nmain: 2 + 3 * 4\n")
        .assert_compiled()
        .assert_exit_code(20);
}

#[test]
fn test_parenthesized_expression() {
    cra("parenthesized", "#entry\nmain: (2 + 3) * 4\n")
        .assert_compiled()
        .assert_exit_code(20);
}

#[test]
fn narrow_integer_shifts_accept_literal_counts() {
    for (name, range, operation, input, expected, masked) in [
        ("left", "0 to 255", "<<", 3, 24, true),
        ("right", "0 to 255", ">>", 192, 24, false),
        ("signed_right", "-128 to 127", ">>", -64, 248, false),
    ] {
        let shifted = if masked {
            format!("((value {operation} 3) & 255)")
        } else {
            format!("(value {operation} 3)")
        };
        let source = format!(
            "Byte is in {range}\nByte has\n    finish(self):\n        return\nshift(value is in {range}) Byte: return {shifted} as Byte\n#entry\nmain: shift({input})\n"
        );
        cra(&format!("literal_shift_{name}"), &source)
            .assert_compiled()
            .assert_exit_code(expected);
    }
}
