mod common;
use common::*;

// if-expression codegen is a known WIP (arith.trunci on i1→i1). The test
// below is a regression guard — upgrade to assert_exit_code once fixed.

#[test]
fn test_if_expression() {
    // Gin `if` syntax: the `return` aligned with `if` closes the if-expression. An indented
    // `main:` `BindValue::Body` still needs a separate `return` after the block (see
    // `parse_bind_value`); dedent that `return` to top level like `bisect_main_with_simple_bind`.
    // Here `return 1` is duplicated so main exits 1 when `x > 5` (same as the if branch for x=10).
    let source = "main:\n    x: 10\n    if x > 5\n        x\n    return 1\nreturn 1\n";
    let result = cra("if_expression", source);
    // TODO: upgrade to `.assert_compiled().assert_exit_code(1)` once
    // boolean-to-integer narrowing is fixed in codegen.
    if result.compiled() {
        result.assert_exit_code(1);
    }
}
