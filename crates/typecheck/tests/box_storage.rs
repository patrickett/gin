use ast::{prelude::FileSemanticOrigin, ty::PackageInstanceKey};
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::transform::{
    PackageTransformOptions, transform_package_with_shared_context_and_package,
};

const ALLOCATOR: &str = r#"use core.alloc Allocation, AllocationResult, Allocator, AllocError
#phantom(unit)
Count(unit) is in 0 to 255
Count(unit) has
    as_num(self ref) is in Self: return (number is in Self: self)
    release(self):
        return
max_count := 255
replace_count(unit, previous Count(unit), next Count(unit)) Count(unit): return next
GlobalAllocator has Allocator
    reserve(self mut, c Count(x)) AllocationResult(x):
        error AllocError: AllocError.OutOfMemory
        return error
    reclaim(self mut, a Allocation(x)):
        Allocation(address, capacity, initialized) := a
        raw: address as @x
        return
    release(self):
        return
"#;

fn expect(source: &str, expected: &[&str]) {
    let package = PackageInstanceKey::workspace("core", "1");
    let files = [
        ("alloc", ALLOCATOR.to_string()),
        ("alloc", include_str!("../../../modules/core/alloc/error.gin").to_string()),
        ("alloc", include_str!("../../../modules/core/alloc/allocator.gin").to_string()),
        ("slice", r#"Byte is in 0 to 255
Byte has
    release(self):
        return
Length is in 0 to 255
Length has
    release(self):
        return
SliceView(element) has
    #private(alloc)
    len Length
    data ref [element; len]
    release(self):
        return
SliceMutView(element) has
    #private(alloc)
    len Length
    data mut [element; len]
    release(self):
        return
"#.to_string()),
        ("alloc.system", "exit_process(status is in 0 to 255):\n    return\n".to_string()),
        ("alloc", include_str!("../../../modules/core/alloc/box.gin").to_string()),
        ("alloc", include_str!("../../../modules/core/alloc/address.gin").to_string()),
        ("ops", "Release has\n    release(self)\n".to_string()),
        ("client", format!("use core.alloc Box, BoxResult, BoxFailure, BoxError, GlobalAllocator, Allocation, Count\nuse core.slice Byte, SliceView, SliceMutView\nread(value ref Byte) Byte: return deref value\n{source}")),
    ].into_iter().map(|(module, source)| {
        let mut parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}\n{source}", parsed.symptoms);
        parsed.ast.semantic_origin = Some(FileSemanticOrigin {
            package: package.clone(),
            module: module.split('.').map(Intern::from_ref).collect(),
        });
        parsed.ast
    }).collect();
    let outputs = transform_package_with_shared_context_and_package(
        files,
        PackageTransformOptions::FULL,
        package,
    );
    let flaws: Vec<_> = outputs
        .typed_asts
        .iter()
        .flat_map(|file| file.all_flaws())
        .collect();
    let mut actual: Vec<_> = flaws.iter().map(|(_, flaw)| flaw.code.slug()).collect();
    actual.sort();
    actual.dedup();
    let mut expected = expected.to_vec();
    expected.sort();
    expected.dedup();
    assert_eq!(actual, expected, "{flaws:?}\n{source}");
}

const CASES: &[(&str, &str, &[&str])] = &[
    (
        "taking_a_box_transfers_the_value_and_invalidates_its_borrow",
        "run(value own Box(Byte)) Byte:\n    saved: value.borrow\n    taken: value.take\n    taken.release\n    return read(saved)\n",
        &["type-use-of-invalidated-reference"],
    ),
    (
        "releasing_a_box_invalidates_its_borrow",
        "run(value Box(Byte)) Byte:\n    saved: value.borrow\n    value.release\n    return read(saved)\n",
        &["type-use-of-invalidated-reference"],
    ),
    (
        "consuming_another_box_preserves_the_first_borrow",
        "run(first Box(Byte), second Box(Byte)) Byte:\n    saved: first.borrow\n    second.release\n    result: read(saved)\n    first.release\n    return result\n",
        &[],
    ),
    (
        "shared_box_access_cannot_supply_a_mutable_reference",
        "run(value ref Box(Byte)) mut Byte: return value.borrow_mut\n",
        &[
            "type-generic-specialization",
            "type-mutable-borrow-from-shared",
        ],
    ),
    (
        "taking_a_box_twice_is_rejected",
        "run(value Box(Byte)) Byte:\n    first: value.take\n    first.release\n    return value.take\n",
        &["type-use-after-move", "type-use-of-moved-value"],
    ),
    (
        "shared_access_does_not_consume_the_value",
        "run(value Box(Byte)) Byte:\n    observed: read(value.borrow)\n    value.release\n    return observed\n",
        &[],
    ),
    (
        "a_consuming_argument_cannot_alias_a_borrow_used_by_the_call",
        "consume(target mut Box(Byte), value own Box(Byte)):\n    value.release\n    observed: read(target.borrow)\n    observed.release\n    return\nrun(value Box(Byte)):\n    stored: value\n    consume(stored, stored)\n    return\n",
        &["type-overlapping-target-group-arguments"],
    ),
    (
        "a_consuming_argument_cannot_alias_a_shared_element_argument",
        "consume(value own Box(Byte), observed ref Byte):\n    value.release\n    result: read(observed)\n    result.release\n    return\nrun(value Box(Byte)):\n    stored: value\n    observed: stored.borrow\n    consume(stored, observed)\n    return\n",
        &["type-overlapping-target-group-arguments"],
    ),
    (
        "a_consuming_argument_preserves_a_disjoint_borrow_argument",
        "consume(value own Box(Byte), observed ref Byte):\n    value.release\n    result: read(observed)\n    result.release\n    return\nrun(first Box(Byte), second Box(Byte)):\n    observed: second.borrow\n    consume(first, observed)\n    second.release\n    return\n",
        &[],
    ),
    (
        "a_box_cannot_be_forgotten_by_its_client",
        "forget(value Box(Byte)):\n    return\n",
        &["type-terminal-value-not-consumed"],
    ),
    (
        "allocation_failure_returns_the_input_and_allocator",
        "recover(failure BoxFailure(Byte, GlobalAllocator)) Byte:\n    parts: failure.into_parts\n    parts.allocator.release\n    return parts.value\n",
        &[],
    ),
    (
        "a_box_can_move_before_a_new_borrow_is_created",
        "move_box(value own Box(Byte)) Box(Byte): return value\nrun(value own Box(Byte)) Byte:\n    moved: move_box(value)\n    observed: read(moved.borrow)\n    moved.release\n    return observed\n",
        &[],
    ),
    (
        "an_imported_factory_preserves_the_failure_inputs",
        "run Byte:\n    attempt: Box.new(Byte, 7 as Byte, GlobalAllocator())\n    return when attempt is\n        Box(value) then value.take\n        BoxFailure(failure) then recover(failure)\nrecover(failure BoxFailure(Byte, GlobalAllocator)) Byte:\n    parts: failure.into_parts\n    parts.allocator.release\n    return parts.value\n",
        &[],
    ),
    (
        "checked_initialization_transfers_storage_on_every_outcome",
        "initialize(value own Byte, allocator own GlobalAllocator, storage Allocation(Byte) and storage.initialized_count = 0 and storage.capacity_count > 0 and storage.address_value > 0 and storage.address_value % #alignment(Byte) = 0) BoxResult(Byte, GlobalAllocator): return Box.initialize(Byte, value, allocator, storage)\n",
        &[],
    ),
    (
        "default_factory_supplies_the_global_allocator",
        "run Byte:\n    attempt: Box.new(Byte, 7 as Byte)\n    return when attempt is\n        Box(value) then value.take\n        BoxFailure(failure) then recover(failure)\nrecover(failure BoxFailure(Byte, GlobalAllocator)) Byte:\n    parts: failure.into_parts\n    parts.allocator.release\n    return parts.value\n",
        &[],
    ),
    (
        "a_client_cannot_forge_a_box_from_an_existing_allocation",
        "forge(a own GlobalAllocator, storage Allocation(Byte)) Box(Byte): return Box(allocator: a, storage)\n",
        &["type-private-member"],
    ),
    (
        "consuming_an_allocation_invalidates_its_element_reference",
        "run(storage Allocation(Byte) and storage.initialized_count = 1, allocator own GlobalAllocator) Byte:\n    saved: storage.borrow(0 as Count(Byte))\n    storage.release(allocator)\n    allocator.release\n    return read(saved)\n",
        &["type-use-of-invalidated-reference"],
    ),
    (
        "borrowed_heap_payloads_are_rejected_until_stored_loans_are_tracked",
        "transfer(storage Allocation(ref Byte)) Allocation(ref Byte): return storage\n",
        &["type-storage-borrowed-payload"],
    ),
    (
        "observing_a_mutable_view_cannot_borrow_an_element_mutably",
        "expose(view ref SliceMutView(Byte)) mut Byte: return mut view.data.[0]\n",
        &["type-mutable-borrow-from-shared"],
    ),
    (
        "shared_access_cannot_duplicate_a_mutable_view",
        "duplicate(value ref SliceMutView(Byte)) SliceMutView(Byte): return deref value\n",
        &["type-move-out-of-borrow"],
    ),
    (
        "a_client_cannot_duplicate_a_box_through_a_shared_reference",
        "duplicate(value ref Box(Byte)) Box(Byte): return deref value\n",
        &["type-storage-authority"],
    ),
    (
        "borrowed_allocator_state_is_rejected_until_result_loans_are_tracked",
        "BorrowedAllocator has\n    state mut Byte\n    release(self):\n        return\ntransfer(value Box(Byte, BorrowedAllocator)) Box(Byte, BorrowedAllocator): return value\n",
        &["type-storage-borrowed-payload"],
    ),
    (
        "indexed_shared_access_cannot_duplicate_a_box",
        "duplicate(value ref [Box(Byte); 1]) Box(Byte): return value.[0]\n",
        &["type-storage-authority"],
    ),
    (
        "shared_access_to_a_record_field_cannot_duplicate_a_box",
        "Holder has\n    boxed Box(Byte)\n    release(self):\n        Holder(boxed) := self\n        boxed.release\n        return\nduplicate(value ref Holder) Box(Byte): return value.boxed\n",
        &["type-storage-authority"],
    ),
    (
        "shared_access_cannot_duplicate_a_record_containing_a_box",
        "Holder has\n    boxed Box(Byte)\n    release(self):\n        Holder(boxed) := self\n        boxed.release\n        return\nduplicate(value ref Holder) Holder: return deref value\n",
        &["type-storage-authority"],
    ),
    (
        "a_raw_cast_cannot_forge_a_record_containing_a_box",
        "Holder has\n    boxed Box(Byte)\n    release(self):\n        Holder(boxed) := self\n        boxed.release\n        return\nforge(address @Holder) Holder: return address as Holder\n",
        &["type-storage-authority"],
    ),
    (
        "generic_shared_access_cannot_duplicate_a_box",
        "duplicate(x, value ref x) x: return deref value\nrun(value ref Box(Byte)) Box(Byte): return duplicate(Box(Byte), value)\n",
        &["type-storage-authority"],
    ),
    (
        "sealing_a_wrapper_does_not_grant_its_foreign_storage_authority",
        "Holder has\n    #private\n    boxed Box(Byte)\n    release(self):\n        Holder(boxed) := self\n        boxed.release\n        return\nforge(address @Holder) Holder: return address as Holder\n",
        &["type-storage-authority"],
    ),
    (
        "a_mutable_reference_alias_cannot_launder_raw_storage_ownership",
        "Holder has\n    #private\n    boxed Box(Byte)\n    release(self):\n        Holder(boxed) := self\n        boxed.release\n        return\nMutable(x) is mut x\nforge(address @Holder) Holder:\n    pointer Mutable(Holder): address as Mutable(Holder)\n    return deref pointer\n",
        &["type-storage-authority", "type-move-out-of-borrow"],
    ),
    (
        "an_owning_storage_field_cannot_be_left_unhandled",
        "Storage has\n    address own Byte\n    finish(self):\n        return\n",
        &["type-terminal-field-not-consumed"],
    ),
    (
        "owning_storage_can_move_between_records",
        "Storage has\n    address own Byte\n    new(address @Byte) Self: return Self(address)\n    into_other(self) Other:\n        Storage(address) := self\n        return Other(address)\nOther has\n    address own Byte\n    finish(self):\n        Other(address) := self\n        raw: address as @Byte\n        return\ntransfer(value own Storage) Other: return value.into_other\n",
        &[],
    ),
    (
        "an_owning_field_requires_the_matching_storage_element_type",
        "Storage has\n    address own Byte\n    finish(self):\n        Storage(address) := self\n        raw: address as @Byte\n        return\nforge(address @Count(Byte)) Storage: return Storage(address)\n",
        &["type-owned-storage-element-mismatch"],
    ),
    (
        "an_owning_field_cannot_be_transferred_through_a_shared_borrow",
        "Storage has\n    address own Byte\n    finish(self):\n        Storage(address) := self\n        raw: address as @Byte\n        return\n    duplicate(self ref) Self: return Self(address: self.address)\n",
        &["type-storage-authority"],
    ),
    (
        "shared_owned_storage_cannot_write_an_element",
        "replace_byte(previous Byte, next Byte) Byte: return next\nStorage has\n    address own Byte\n    finish(self):\n        Storage(address) := self\n        raw: address as @Byte\n        return\n    write(self ref):\n        self.address.[0] :: replace_byte(self.address.[0], 1)\n        return\n",
        &["type-rebind-immutable", "type-mutable-borrow-from-shared"],
    ),
    (
        "shared_access_to_a_record_field_cannot_release_its_box",
        "Holder has\n    boxed Box(Byte)\n    release(self):\n        Holder(boxed) := self\n        boxed.release\n        return\nrelease_borrowed(value ref Holder):\n    value.boxed.release\n    return\n",
        &["type-storage-authority"],
    ),
];

#[test]
fn box_storage_cases() {
    let mut failures = Vec::new();
    for &(name, source, expected) in CASES {
        if std::panic::catch_unwind(|| expect(source, expected)).is_err() {
            failures.push(name);
        }
    }
    assert!(failures.is_empty(), "failing cases: {failures:?}");
}

#[test]
fn a_shared_reference_cannot_take_or_release_its_box() {
    expect(
        "take_shared(value ref Box(Byte)) Byte: return value.take\n",
        &["type-generic-specialization", "type-move-out-of-borrow"],
    );
    expect(
        "release_shared(value ref Box(Byte)):\n    value.release\n    return\n",
        &["type-generic-specialization", "type-move-out-of-borrow"],
    );
}

#[test]
fn storage_addresses_preserve_the_unsigned_pointer_range() {
    for (triple, width) in [
        ("wasm32-unknown-unknown", 32_u32),
        ("aarch64-apple-darwin", 64),
        ("x86_64-unknown-linux-gnu", 64),
    ] {
        let target = flask::CompileTarget::Concrete(flask::TargetTriple::parse(triple).unwrap());
        let mut files = vec![
            include_str!("../../../modules/core/alloc/address.gin")
                .parse_source_full()
                .ast,
        ];
        assert!(
            typecheck::prepare_package_asts(&mut files, &target)
                .iter()
                .all(Vec::is_empty)
        );
        let outputs = typecheck::transform::transform_package_with_shared_context(
            files,
            PackageTransformOptions::FULL.with_compile_target(target),
        );
        let typed = &outputs.typed_asts[0];
        assert!(
            typed.all_flaws().is_empty(),
            "{triple}: {:?}",
            typed.all_flaws()
        );
        let address =
            &typed.tags[&typecheck::TagId(Intern::from_ref("StorageAddress"))].resolved_ty;
        assert_eq!(
            typed.type_registry.integer_width_for_type(address),
            Some(width)
        );
        let range = typed
            .type_registry
            .integer_validity_for_type(address)
            .unwrap()
            .domain();
        assert!(range.contains(i256::I256::from(0)));
        assert!(range.contains((i256::I256::from(1) << width) - i256::I256::from(1)));
        assert!(!range.contains(i256::I256::from(-1)));
        assert!(!range.contains(i256::I256::from(1) << width));
    }
}

#[test]
fn a_client_cannot_read_or_mutate_the_storage_representation() {
    expect(
        "read_raw(storage ref Allocation(Byte)) @Byte: return storage.address\n",
        &["type-private-member"],
    );
    expect(
        "read_pointer(storage ref Allocation(Byte)) ref @Byte: return ref storage.address\n",
        &["type-private-member"],
    );
}

#[test]
fn views_forward_through_an_owning_record_and_follow_its_consumption() {
    let holder = "Holder has\n    storage Allocation(Byte)\n    view(self ref) SliceView(Byte): return self.storage.view\n    release(self):\n        Holder(storage) := self\n        region: storage\n        allocator: GlobalAllocator()\n        region.release_values\n        region.reclaim(allocator)\n        allocator.release\n        return\n";
    expect(holder, &[]);
    expect(
        &format!(
            "{holder}run(owner Holder) Byte:\n    saved: owner.view\n    owner.release\n    result: saved.data.[0]\n    saved.release\n    return result\n"
        ),
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn a_raw_pointer_cannot_forge_an_owner_through_a_containing_record() {
    let holder = "Holder has\n    boxed Box(Byte)\n    release(self):\n        Holder(boxed) := self\n        boxed.release\n        return\n";
    for body in ["deref pointer", "pointer.[0]"] {
        expect(
            &format!(
                "{holder}forge(pointer @Holder) Holder: {return_prefix}{body}\n",
                return_prefix = if body.starts_with('\n') {
                    ""
                } else {
                    "return "
                }
            ),
            &["type-storage-authority"],
        );
    }
}

#[test]
fn a_borrow_cannot_establish_storage_ownership() {
    for parameter in ["value ref Byte", "value mut Byte", "value Byte"] {
        expect(
            &format!(
                "Storage has\n    address own Byte\n    finish(self):\n        Storage(address) := self\n        raw: address as @Byte\n        return\nforge({parameter}) Storage: return Storage(address: value)\n"
            ),
            &["type-owned-storage-origin"],
        );
    }
}

#[test]
fn owning_storage_elements_require_mutable_access_to_move() {
    for (permission, expected) in [("ref", &["type-move-out-of-borrow"][..]), ("mut", &[][..])] {
        expect(
            &format!(
                "Resource has\n    value Byte\n    release(self):\n        return\nStorage has\n    address own Resource\n    finish(self):\n        Storage(address) := self\n        raw: address as @Resource\n        return\n    take(self {permission}) Resource: return self.address.[0]\n"
            ),
            expected,
        );
    }
}

#[test]
fn shared_owned_storage_cannot_provide_a_mutable_element_borrow() {
    for (element, borrowed) in [
        ("Byte", "self.address.[0]"),
        ("Resource", "self.address.[0].value"),
    ] {
        expect(
            &format!(
                "Resource has\n    value Byte\n    release(self):\n        return\nStorage has\n    address own {element}\n    finish(self):\n        Storage(address) := self\n        raw: address as @{element}\n        return\n    borrow_mut(self ref) mut Byte: return mut {borrowed}\n"
            ),
            &["type-mutable-borrow-from-shared"],
        );
    }
}

#[test]
fn owning_storage_fields_must_be_stored() {
    for declaration in [
        "Invalid has\n    pointer own Byte: 0\n",
        "Invalid has\n    pointer own Byte := 0\n",
    ] {
        expect(
            &format!(
                "{declaration}    finish(self):\n        Invalid(pointer) := self\n        raw: pointer as @Byte\n        return\n"
            ),
            &["type-invalid-owned-storage-field"],
        );
    }
}
