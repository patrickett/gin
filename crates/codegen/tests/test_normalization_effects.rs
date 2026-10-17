mod support;

const DECLARATIONS: &str =
    include_str!("../../test-fixtures/fixtures/normalization_statement_effects.gin");

#[test]
#[allow(unsafe_code)]
fn binding_statements_preserve_native_effects_before_a_constant_return() {
    for (statement, mutates) in [("5", false), ("grow(value)", true)] {
        let source = format!(
            "{DECLARATIONS}\ncheck(value mut Count) Count:\n    local Count:\n        {statement}\n        7\n    return local\n"
        );
        let engine = support::native_engine_for_source(&source, "normalization.gin", "check");
        for initial in [0_u8, 7, 15, 16, 255] {
            let mut state = initial;
            let mut pointer = &mut state as *mut u8;
            let mut result = 0_u8;
            unsafe {
                engine.invoke_packed(&mut [
                    (&mut pointer as *mut *mut u8).cast(),
                    (&mut result as *mut u8).cast(),
                ])
            }
            .unwrap();
            assert_eq!(state, if mutates { 255 } else { initial });
            assert_eq!(result, 7);
        }
    }
}
