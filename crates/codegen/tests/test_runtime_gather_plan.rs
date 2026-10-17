#[path = "../benches/runtime_gather/support.rs"]
mod native;
mod support;

#[test]
fn stored_bounds_remove_checks_and_raw_indices_keep_them() {
    let (llvm, diagnostics) = support::codegen_to_llvm_ir(native::SOURCE, "runtime_gather.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for (name, checked) in [("proven_at", false), ("checked_at", true)] {
        let body = support::llvm_function_body(&llvm, name);
        assert!(body.contains("getelementptr"), "{body}");
        assert_eq!(body.contains("llvm.trap"), checked, "{body}");
        assert_eq!(body.contains("icmp"), checked, "{body}");
    }
}

#[test]
fn ten_thousand_indices_reuse_their_bounds_across_one_hundred_timesteps() {
    let kernels = native::Kernels::new();
    let indices = (0..native::INDEX_CAPACITY)
        .map(|i| ((i * 7919 + 17) % native::VALUE_CAPACITY) as u16)
        .collect::<Vec<_>>();
    let input = native::Input::new(&indices);
    assert_eq!(input.indices(), indices);
    let plan = kernels
        .prepare(&input, native::VALUE_CAPACITY as u16)
        .unwrap();
    assert_eq!(plan.indices(), indices);
    assert_eq!(
        plan.extents(),
        vec![native::VALUE_CAPACITY as u16; indices.len()]
    );
    let readers = [
        native::Mode::CheckedCompact,
        native::Mode::CheckedWitness,
        native::Mode::Prepared,
    ]
    .map(|mode| kernels.selected_sum(mode));
    for timestep in 0..100 {
        let values: Box<[u8; native::VALUE_CAPACITY]> = (0..native::VALUE_CAPACITY)
            .map(|i| ((i * 31 + timestep * 17) % 256) as u8)
            .collect::<Vec<_>>()
            .into_boxed_slice()
            .try_into()
            .unwrap();
        let expected = indices
            .iter()
            .map(|index| u64::from(values[usize::from(*index)]))
            .sum::<u64>();
        for reader in &readers {
            assert_eq!(reader(&plan, &input, &values), expected);
        }
    }
}

#[test]
fn validation_handles_empty_boundary_and_partially_invalid_collections() {
    let kernels = native::Kernels::new();
    let empty = native::Input::new(&[]);
    let plan = kernels.prepare(&empty, 0).unwrap();
    let values: Box<[u8; native::VALUE_CAPACITY]> = vec![255; native::VALUE_CAPACITY]
        .into_boxed_slice()
        .try_into()
        .unwrap();
    for mode in [
        native::Mode::CheckedCompact,
        native::Mode::CheckedWitness,
        native::Mode::Prepared,
    ] {
        assert_eq!(kernels.selected_sum(mode)(&plan, &empty, &values), 0);
    }
    for length in [1, 17, native::INDEX_CAPACITY] {
        let valid = vec![127; length];
        assert!(kernels.prepare(&native::Input::new(&valid), 128).is_some());
        for slot in [0, length / 2, length - 1] {
            let mut invalid = valid.clone();
            invalid[slot] = 128;
            assert!(
                kernels
                    .prepare(&native::Input::new(&invalid), 128)
                    .is_none()
            );
            invalid[slot] = u16::MAX;
            assert!(
                kernels
                    .prepare(&native::Input::new(&invalid), 128)
                    .is_none()
            );
        }
    }
    assert!(kernels.prepare(&native::Input::new(&[0]), 0).is_none());
}

#[test]
fn plans_copy_indices_and_support_variable_visible_lengths() {
    let kernels = native::Kernels::new();
    let values: Box<[u8; native::VALUE_CAPACITY]> = (0..native::VALUE_CAPACITY)
        .map(|i| (i % 256) as u8)
        .collect::<Vec<_>>()
        .into_boxed_slice()
        .try_into()
        .unwrap();
    for length in [1, 7, 64, 9999] {
        let indices = (0..length).map(|i| (i % 128) as u16).collect::<Vec<_>>();
        let mut input = native::Input::new(&indices);
        let plan = kernels.prepare(&input, 128).unwrap();
        input.indices_mut().fill(u16::MAX);
        assert_eq!(plan.indices(), indices);
        assert_eq!(plan.extents(), vec![128; length]);
        let expected = indices
            .iter()
            .map(|index| u64::from(values[usize::from(*index)]))
            .sum::<u64>();
        assert_eq!(
            kernels.selected_sum(native::Mode::Prepared)(&plan, &input, &values),
            expected
        );
    }
}

#[test]
fn losing_a_bound_or_extent_contract_restores_checks() {
    for source in [
        native::SOURCE.replace("position Count and < extent", "position Count"),
        native::SOURCE.replace(" and view.length = index.extent", ""),
    ] {
        let (llvm, diagnostics) = support::codegen_to_llvm_ir(&source, "unproved_gather.gin");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let body = support::llvm_function_body(&llvm, "proven_at");
        assert!(body.contains("llvm.trap"), "{body}");
    }
}
