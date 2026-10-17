mod support;

#[path = "../benches/gather_plan/support.rs"]
mod native;

const PLAN: &str = include_str!("../../test-fixtures/fixtures/validated_gather_plan.gin");

#[test]
fn stored_index_bounds_remove_checks_from_separate_accessors() {
    let parsed = parser::query::SourceParseExt::parse_source_full(PLAN);
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = typecheck::transform::transform_file(parsed.ast, typecheck::FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let (llvm, diagnostics) = support::codegen_to_llvm_ir(PLAN, "gather_plan.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{llvm}");
    for name in ["read_first", "read_second", "read_third", "read_fourth"] {
        let body = support::llvm_function_body(&llvm, name);
        assert!(body.contains("getelementptr"), "{body}");
        assert!(!body.contains("icmp"), "{body}");
        assert!(!body.contains("llvm.trap"), "{body}");
    }
}

#[test]
fn losing_a_stored_index_bound_restores_runtime_checking() {
    let source = PLAN.replace("first Count and < length", "first Count");
    let (llvm, diagnostics) = support::codegen_to_llvm_ir(&source, "unbounded_plan.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{llvm}");
    let body = support::llvm_function_body(&llvm, "read_first");
    assert!(body.contains("llvm.trap"), "{body}");
}

#[test]
fn losing_the_view_extent_contract_restores_runtime_checking() {
    let source = PLAN.replace(" and view.length = plan.length", "");
    let (llvm, diagnostics) = support::codegen_to_llvm_ir(&source, "unmatched_extent.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{llvm}");
    for name in ["read_first", "read_second", "read_third", "read_fourth"] {
        let body = support::llvm_function_body(&llvm, name);
        assert!(body.contains("llvm.trap"), "{body}");
    }
}

#[test]
fn native_validation_rejects_empty_and_out_of_range_plans() {
    let kernels = native::Kernels::new();
    assert!(kernels.prepare(0, [0; 4]).is_none());
    for slot in 0..4 {
        let mut indices = [3, 0, 2, 3];
        indices[slot] = 4;
        assert!(kernels.prepare(4, indices).is_none());
        indices[slot] = u16::MAX;
        assert!(kernels.prepare(4, indices).is_none());
    }
}

#[test]
fn native_plans_copy_indices_and_reuse_them_on_different_buffers() {
    let kernels = native::Kernels::new();
    for length in [1, 4, 128, native::CAPACITY as u16] {
        let mut indices = [length - 1, 0, length / 2, length - 1];
        let plan = kernels.prepare(length, indices).unwrap();
        assert_eq!(plan.length(), length);
        assert_eq!(plan.indices(), indices);
        indices.fill(u16::MAX);
        for row in 0..16 {
            let values = std::array::from_fn(|column| ((column * 31 + row * 17) % 256) as u8);
            let expected = plan
                .indices()
                .iter()
                .map(|index| u64::from(values[usize::from(*index)]))
                .sum::<u64>();
            for _ in 0..8 {
                assert_eq!(kernels.sum_prepared(&plan, &values), expected);
                assert_eq!(kernels.sum_checked(&plan, &values), expected);
                assert_eq!(kernels.selected_sum(true)(&plan, &values), expected);
                assert_eq!(kernels.selected_sum(false)(&plan, &values), expected);
            }
        }
    }
}
