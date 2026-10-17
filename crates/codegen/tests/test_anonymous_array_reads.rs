mod support;

#[test]
#[allow(unsafe_code)]
fn domain_reads_preserve_owned_and_borrowed_array_elements_across_iterations() {
    for qualifier in ["", "ref "] {
        let source = format!(
            r#"Result is in 0 to 65535
Result has finish(self): return
read(source {qualifier}[in 0 to 255; 2]) Result:
    last is in 0 to 255: 0
    for index in 0 to 1
        value is in 0 to 255: source.[index]
        last:: value
    first is in 0 to 255: source.[0]
    return (first + last) as Result
compute(left in 0 to 255, right in 0 to 255) Result:
    source [in 0 to 255; 2]: [left, right]
    return read(source)
"#
        );
        let engine = support::native_engine_for_source(&source, "array_reads.gin", "compute");
        for (mut left, mut right) in [(0u8, 0u8), (3, 17), (128, 255), (255, 255)] {
            let mut result = 0u16;
            unsafe {
                engine.invoke_packed(&mut [
                    (&mut left as *mut u8).cast(),
                    (&mut right as *mut u8).cast(),
                    (&mut result as *mut u16).cast(),
                ])
            }
            .unwrap();
            assert_eq!(result, u16::from(left) + u16::from(right), "{qualifier}");
        }
    }
}
