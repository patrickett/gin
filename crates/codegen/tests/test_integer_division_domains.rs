mod support;

#[allow(unsafe_code)]
fn check<L: Copy + std::fmt::Debug, R: Copy + std::fmt::Debug>(
    source: &str,
    cases: &[(L, R, i64)],
) {
    let engine = support::native_engine_for_source(source, "division.gin", "compute");
    for &(left, right, expected) in cases {
        let mut left_input = left;
        let mut right_input = right;
        let mut result = 0i64;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut left_input as *mut L).cast(),
                (&mut right_input as *mut R).cast(),
                (&mut result as *mut i64).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected, "left={left:?}, right={right:?}");
    }
}

#[test]
fn a_small_remainder_keeps_the_complete_dividend() {
    for (operator, cases) in [
        ("%", vec![(255u8, 10u8, 5), (128, 3, 2), (0, 1, 0)]),
        ("/", vec![(255, 10, 25), (128, 3, 42), (0, 1, 0)]),
    ] {
        let source = format!(
            "compute(left in 0 to 255, right in 1 to 10) \
             is in -9223372036854775808 to 9223372036854775807:\n    return left {operator} right\n"
        );
        check(&source, &cases);
    }
}

#[test]
fn negative_divisors_select_signed_arithmetic_for_unsigned_dividends() {
    for (operator, expected) in [("/", -25), ("%", 5)] {
        let source = format!(
            "compute(left in 0 to 255, right in -10 to -1) \
             is in -9223372036854775808 to 9223372036854775807:\n    return left {operator} right\n"
        );
        check(&source, &[(255u8, -10i8, expected), (0, -1, 0)]);
    }
}

#[test]
fn signed_remainders_preserve_the_dividend_sign_and_minimum_boundary() {
    let source = concat!(
        "compute(left in -9223372036854775808 to 9223372036854775807, ",
        "right in -10 to -1) is in -9223372036854775808 to 9223372036854775807:\n",
        "    return left % right\n",
    );
    check(
        source,
        &[(i64::MIN, -1i8, 0), (i64::MIN, -10, -8), (123, -10, 3)],
    );
}

#[test]
fn wide_remainders_preserve_signed_and_unsigned_boundaries() {
    let signed = format!(
        "compute(left in {} to {}, right in -10 to -1) \
         is in -9223372036854775808 to 9223372036854775807:\n    return left % right\n",
        i128::MIN,
        i128::MAX,
    );
    check(&signed, &[(i128::MIN, -1i8, 0), (i128::MIN, -10, -8)]);
    let unsigned = format!(
        "compute(left in 0 to {}, right in -10 to -1) \
         is in -9223372036854775808 to 9223372036854775807:\n    return left % right\n",
        u128::MAX,
    );
    check(&unsigned, &[(u128::MAX, -1i8, 0), (u128::MAX, -10, 5)]);
}
