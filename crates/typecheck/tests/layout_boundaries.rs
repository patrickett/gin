use typecheck::layout::{LayoutTarget, TargetLayout};

#[test]
fn maximum_alignment_handles_index_widths_larger_than_u64() {
    for (index_bits, expected) in [
        (64, 1u64 << 62),
        (65, 1u64 << 63),
        (66, u64::MAX),
        (128, u64::MAX),
        (u32::MAX, u64::MAX),
    ] {
        let layout = TargetLayout::with_address_layout(LayoutTarget::X86_64, 8, 8, index_bits)
            .expect("valid address layout");
        assert_eq!(layout.max_alignment(), expected);
    }
}
