mod common;
use common::*;

#[test]
fn test_less_than() {
    cra("cmp_lt", "main: 1 < 2\n").assert_compiled();
}

#[test]
fn test_greater_than() {
    cra("cmp_gt", "main: 2 > 1\n").assert_compiled();
}

#[test]
fn test_equality() {
    cra("cmp_eq", "main: 1 == 1\n").assert_compiled();
}
