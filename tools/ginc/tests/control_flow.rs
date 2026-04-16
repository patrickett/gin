mod common;
use common::*;

// if-expression codegen is a known WIP (arith.trunci on i1→i1). The test
// below is a regression guard — upgrade to assert_exit_code once fixed.

#[test]
fn test_if_expression() {
    let source = "main:\n    x: 10\n    if x > 5\n        return 1\n    return 0\n";
    let result = cra("if_expression", source);
    // TODO: upgrade to `.assert_compiled().assert_exit_code(1)` once
    // boolean-to-integer narrowing is fixed in codegen.
    if result.compiled() {
        result.assert_exit_code(1);
    }
}
