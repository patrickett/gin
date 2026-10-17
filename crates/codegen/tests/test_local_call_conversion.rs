mod support;

#[allow(unsafe_code)]
fn check<T: Copy + std::fmt::Debug>(source: &str, cases: &[(T, T, i16)]) {
    let engine = support::native_engine_for_source(source, "local_conversion.gin", "convert");
    for &(left, right, expected) in cases {
        let mut left_input = left;
        let mut right_input = right;
        let mut result = 0i16;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut left_input as *mut T).cast(),
                (&mut right_input as *mut T).cast(),
                (&mut result as *mut i16).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected, "left={left:?}, right={right:?}");
    }
}

#[test]
fn local_calls_zero_extend_widened_unsigned_arguments_into_signed_parameters() {
    let source = r#"convert(left in 0 to 255, right in 0 to 255):
    widen(value in -32768 to 32767): return value
    return widen(left + right)
"#;
    check(
        source,
        &[(255u8, 255u8, 510), (128, 1, 129), (255, 1, 256), (0, 0, 0)],
    );
}

#[test]
fn local_calls_sign_extend_widened_signed_arguments() {
    let source = r#"convert(left in -128 to 127, right in -128 to 127):
    widen(value in -32768 to 32767): return value
    return widen(left + right)
"#;
    check(
        source,
        &[
            (-128i8, -128i8, -256),
            (-128, 1, -127),
            (127, 127, 254),
            (0, 0, 0),
        ],
    );
}

#[test]
#[allow(unsafe_code)]
fn unequal_operand_widths_preserve_the_full_native_product() {
    let source = concat!(
        "product(quantity in 1 to 10, price in 0 to 10000) is in 0 to 4294967295:\n",
        "    return quantity * price\n",
    );
    let engine = support::native_engine_for_source(source, "product.gin", "product");
    for (quantity, price, expected) in [(3u8, 125u16, 375u32), (10, 10000, 100000), (1, 0, 0)] {
        let mut quantity_input = quantity;
        let mut price_input = price;
        let mut result = 0u32;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut quantity_input as *mut u8).cast(),
                (&mut price_input as *mut u16).cast(),
                (&mut result as *mut u32).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected, "quantity={quantity}, price={price}");
    }
}
