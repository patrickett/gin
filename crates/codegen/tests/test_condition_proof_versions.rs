mod support;

const DECLARATIONS: &str = include_str!("../../test-fixtures/fixtures/callable_small_proof.gin");

#[test]
#[allow(unsafe_code)]
fn fresh_callable_proofs_preserve_native_values_and_mutation_order() {
    for (condition, mutates) in [
        ("small(value) is True", false),
        ("grow(value) is True and small(value) is True", true),
    ] {
        let source = format!(
            "{DECLARATIONS}\nnarrow(value mut is in 0 to 255) Nibble:\n    when {condition} then\n        number is in 0 to 255: value\n        return number as Nibble\n    return 0\n"
        );
        let engine = support::native_engine_for_source(&source, "proof.gin", "narrow");
        for initial in [0_u8, 7, 15, 16, 255] {
            let mut state = initial;
            let mut pointer = &mut state as *mut u8;
            let mut result = 255_u8;
            unsafe {
                engine.invoke_packed(&mut [
                    (&mut pointer as *mut *mut u8).cast(),
                    (&mut result as *mut u8).cast(),
                ])
            }
            .unwrap();
            assert_eq!(state, if mutates { 255 } else { initial });
            assert_eq!(
                result,
                if !mutates && initial <= 15 {
                    initial
                } else {
                    0
                }
            );
        }
    }
}
