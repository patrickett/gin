mod support;

#[allow(unsafe_code)]
fn check<L, R, O>(source: &str, cases: &[(L, R, O)])
where
    L: Copy + std::fmt::Debug,
    R: Copy + std::fmt::Debug,
    O: Copy + Default + PartialEq + std::fmt::Debug,
{
    let engine = support::native_engine_for_source(source, "shift.gin", "compute");
    for &(left, right, expected) in cases {
        let mut left_input = left;
        let mut right_input = right;
        let mut result = O::default();
        unsafe {
            engine.invoke_packed(&mut [
                (&mut left_input as *mut L).cast(),
                (&mut right_input as *mut R).cast(),
                (&mut result as *mut O).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected, "left={left:?}, right={right:?}");
    }
}

#[test]
fn unsigned_shifts_preserve_the_widened_result() {
    let source = concat!(
        "compute(left in 0 to 255, right in 0 to 8) is in 0 to 65535:\n",
        "    return left << right\n",
    );
    check(
        source,
        &[(255u8, 8u8, 65280u16), (128, 1, 256), (255, 0, 255)],
    );
}

#[test]
fn signed_shifts_extend_the_source_sign() {
    let source = concat!(
        "compute(left in -128 to 127, right in 0 to 8) is in -32768 to 32767:\n",
        "    return left << right\n",
    );
    check(
        source,
        &[
            (-128i8, 8u8, -32768i16),
            (-1, 8, -256),
            (127, 8, 32512),
            (-128, 0, -128),
        ],
    );
}

#[test]
fn zero_shifts_preserve_large_counts_without_llvm_overshift() {
    let source = concat!(
        "compute(left in 0 to 0, right in 0 to 255) is in 0 to 18446744073709551615:\n",
        "    return left << right\n",
    );
    check(
        source,
        &[(0u8, 0u8, 0u64), (0, 64, 0), (0, 128, 0), (0, 255, 0)],
    );
}

#[test]
fn guarded_shift_counts_keep_their_original_representation() {
    let source = concat!(
        "compute(left in 0 to 1, right in 0 to 65535):\n",
        "    when right > 63 then\n        return 0\n",
        "    return left << right\n",
    );
    check(
        source,
        &[
            (1u8, 63u16, 1u64 << 63),
            (1, 0, 1),
            (0, 63, 0),
            (1, 65535, 0),
        ],
    );
}

#[test]
fn full_unsigned_sources_widen_to_the_complete_shift_result() {
    let source = format!(
        "compute(left in 0 to {}, right in 0 to 63) is in 0 to {}:\n    return left << right\n",
        u64::MAX,
        u128::MAX,
    );
    check(
        &source,
        &[
            (u64::MAX, 63u8, u128::from(u64::MAX) << 63),
            (u64::MAX, 0, u128::from(u64::MAX)),
        ],
    );
}

#[test]
fn right_shifts_preserve_the_complete_unsigned_source() {
    let source = concat!(
        "compute(left in 0 to 255, right in 3 to 3) is in 0 to 255:\n",
        "    return left >> right\n",
    );
    check(source, &[(255u8, 3u8, 31u8), (128, 3, 16), (0, 3, 0)]);
}

#[test]
fn right_shifts_preserve_the_signed_source_and_sign_extension() {
    let source = concat!(
        "compute(left in -128 to 127, right in 3 to 255) is in -128 to 127:\n",
        "    return left >> right\n",
    );
    check(
        source,
        &[
            (-128i8, 3u8, -16i8),
            (-128, 255, -1),
            (-1, 255, -1),
            (127, 3, 15),
        ],
    );
}

#[test]
fn right_shifts_preserve_large_counts_and_produce_zero_past_source_width() {
    let source = concat!(
        "compute(left in 0 to 255, right in 0 to 65535) is in 0 to 255:\n",
        "    when right > 255 then\n        return 0\n",
        "    return left >> right\n",
    );
    check(
        source,
        &[
            (255u8, 3u16, 31u8),
            (255, 7, 1),
            (255, 8, 0),
            (255, 255, 0),
            (0, 255, 0),
            (255, 65535, 0),
        ],
    );
}

#[test]
fn zero_right_shifts_do_not_overshift_the_result_width() {
    let source = concat!(
        "compute(left in 0 to 0, right in 0 to 255) is in 0 to 255:\n",
        "    return left >> right\n",
    );
    check(source, &[(0u8, 0u8, 0u8), (0, 255, 0)]);
}
