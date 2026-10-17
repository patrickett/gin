#![cfg(any(
    all(target_arch = "aarch64", target_os = "macos"),
    all(target_arch = "x86_64", target_os = "linux")
))]

use test_fixtures::semantic_fixtures::INDEXED_COPY;

mod support;

#[test]
#[allow(unsafe_code)]
fn native_copy_updates_only_the_requested_bytes() {
    let engine = support::native_engine_for_source(INDEXED_COPY, "indexed_copy.gin", "copy");
    for count in 0..=4 {
        let source: [u8; 4] = [3, 17, 128, 255];
        let mut destination: [u8; 4] = [41; 4];
        let mut source_pointer = source.as_ptr();
        let mut destination_pointer = destination.as_mut_ptr();
        let mut count_argument = count as u8;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut source_pointer as *mut *const u8).cast(),
                (&mut destination_pointer as *mut *mut u8).cast(),
                (&mut count_argument as *mut u8).cast(),
            ])
        }
        .unwrap();
        let mut expected = [41; 4];
        expected[..count as usize].copy_from_slice(&source[..count as usize]);
        assert_eq!(destination, expected);
        assert_eq!(source, [3, 17, 128, 255]);
    }
}

#[test]
#[allow(unsafe_code)]
fn native_copy_reads_the_updated_local_storage_with_compact_indices() {
    let engine = support::native_engine_for_source(INDEXED_COPY, "indexed_copy.gin", "read_copied");
    for mut count in 0u8..=4 {
        for mut index in 0u8..4 {
            let mut result = 0u8;
            unsafe {
                engine.invoke_packed(&mut [
                    (&mut count as *mut u8).cast(),
                    (&mut index as *mut u8).cast(),
                    (&mut result as *mut u8).cast(),
                ])
            }
            .unwrap();
            assert_eq!(
                result,
                if index < count {
                    [3, 17, 128, 255][index as usize]
                } else {
                    41
                }
            );
        }
    }
}

#[test]
#[allow(unsafe_code)]
fn native_copy_scales_wide_elements_once() {
    let source = INDEXED_COPY.replace("0 to 255", "0 to 65535");
    let engine = support::native_engine_for_source(&source, "indexed_copy.gin", "copy");
    for mut count in 0u8..=4 {
        let source = [3u16, 257, 32768, 65535];
        let mut destination = [41u16; 4];
        let mut source_pointer = source.as_ptr();
        let mut destination_pointer = destination.as_mut_ptr();
        unsafe {
            engine.invoke_packed(&mut [
                (&mut source_pointer as *mut *const u16).cast(),
                (&mut destination_pointer as *mut *mut u16).cast(),
                (&mut count as *mut u8).cast(),
            ])
        }
        .unwrap();
        let mut expected = [41u16; 4];
        expected[..count as usize].copy_from_slice(&source[..count as usize]);
        assert_eq!(destination, expected);
    }
}
