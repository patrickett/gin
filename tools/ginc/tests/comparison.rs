mod common;
use common::*;

#[test]
fn test_less_than() {
    cra("cmp_lt", "#entry\nmain: when 1 < 2 then 0 else 1\n").assert_compiled();
}

#[test]
fn test_greater_than() {
    cra("cmp_gt", "#entry\nmain: when 2 > 1 then 0 else 1\n").assert_compiled();
}

// Equality test removed: `==` is no longer valid syntax.
// Gin uses `is` for all equality (both compile-time patterns and runtime values).
// Once runtime `a is b` expression support is added, this test will be rewritten.
