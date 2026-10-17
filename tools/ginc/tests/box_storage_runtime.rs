mod common;
use common::compile_exe_checked;
use ginc::cli::Profile;
use ginc::driver::{CompilePackage, CompilerDriver, SourceFile, SourcePackage};
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn core_box_initializes_borrows_transfers_and_reclaims() {
    let package = TempPackage::new("box_storage");
    write_heap_fixture(&package);
    let input = package.write(
        "main.gin",
        include_str!("../../../crates/test-fixtures/fixtures/box_storage_core_box.gin"),
    );
    compile_and_run(&package, &input);
}

#[test]
fn slot_swap_and_take_preserve_owned_values() {
    let package = TempPackage::new("slot_storage");
    write_heap_fixture(&package);
    package.write(
        "alloc/slot.gin",
        include_str!("../../../modules/core/alloc/slot.gin"),
    );
    let input = package.write(
        "main.gin",
        r#"use core.alloc Slot, full, swap, take
use core.slice Byte
Resource has
    value Byte
    release(self):
        return
read_and_release(slot Slot(Resource)) Byte:
    return when slot is
        Empty then 0
        Full(value) then consume(value)
consume(value Resource) Byte:
    result is in Byte: value.value
    value.release
    return result as Byte
#entry
main:
    slot: full(Resource, Resource(value: 7))
    old is in Byte: read_and_release(swap(Resource, slot, full(Resource, Resource(value: 11))))
    current is in Byte: read_and_release(take(Resource, slot))
    empty is in Byte: read_and_release(slot)
    return when old = 7 and current = 11 and empty = 0 then 0 else 1
"#,
    );
    compile_and_run(&package, &input);
}

#[test]
fn allocation_rejects_invalid_operations_at_compile_time() {
    let header = r#"use core.alloc Allocation, AllocError, AllocationResult, Count, GlobalAllocator
use core.slice Byte
unwrap(attempt AllocationResult(Byte)) Allocation(Byte):
    return when attempt is
        Allocation(storage) then storage
        AllocError(error) then failed(error)
failed(error AllocError) Allocation(Byte): return failed(error)
read(value ref Byte) Byte: return deref value
#entry
main:
    global: GlobalAllocator()
    storage: unwrap(global.reserve(1 as Count(Byte)))
    when storage.initialized_count.as_num > 0 or not (storage.capacity_count.as_num = 1) then
        storage.release(global)
        global.release
        return 1
"#;
    for (name, operations) in [
        (
            "uninitialized",
            "    observed: read(storage.borrow(0 as Count(Byte)))\n    storage.reclaim(global)\n",
        ),
        (
            "out_of_bounds",
            "    storage.initialize(1 as Count(Byte), 7 as Byte)\n    storage.release_values\n    storage.reclaim(global)\n",
        ),
        (
            "overwrite",
            "    storage.initialize(0 as Count(Byte), 7 as Byte)\n    storage.initialize(0 as Count(Byte), 9 as Byte)\n    storage.release_values\n    storage.reclaim(global)\n",
        ),
        (
            "moved_out",
            "    storage.initialize(0 as Count(Byte), 7 as Byte)\n    value: storage.take_last\n    value.release\n    observed: read(storage.borrow(0 as Count(Byte)))\n    storage.reclaim(global)\n",
        ),
        (
            "live_reclaim",
            "    storage.initialize(0 as Count(Byte), 7 as Byte)\n    storage.reclaim(global)\n",
        ),
        (
            "live_raw_reclaim",
            "    storage.initialize(0 as Count(Byte), 7 as Byte)\n    global.reclaim(storage)\n",
        ),
    ] {
        let package = TempPackage::new(&format!("allocation_guard_{name}"));
        write_heap_fixture(&package);
        let input = package.write(
            "main.gin",
            &format!("{header}{operations}    global.release\n    return 0\n"),
        );
        let mut driver = CompilerDriver::default();
        let source = std::fs::read_to_string(&input).unwrap();
        let parsed = driver
            .parse(SourcePackage::new(
                &input,
                vec![SourceFile::new(&input, source)],
            ))
            .unwrap();
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&host_target_triple()).unwrap(),
        );
        let checked =
            driver.check(CompilePackage::new(parsed, target).with_import_resolution(true));
        let front_end: Vec<_> = checked
            .files()
            .iter()
            .flat_map(|file| file.front_end_diagnostics())
            .collect();
        assert!(front_end.is_empty(), "{name}: {front_end:?}");
        let flaws: Vec<_> = checked
            .files()
            .iter()
            .flat_map(|file| file.typecheck_diagnostics())
            .filter(|diagnostic| diagnostic.category == diagnostic::Category::Flaw)
            .collect();
        assert!(
            flaws
                .iter()
                .any(|flaw| flaw.code.slug() == "type-parameter-refinement-failed"),
            "{name}: {flaws:?}"
        );
    }
}

#[test]
fn list_capacity_preconditions_require_proofs_at_call_sites() {
    for (name, body, expected) in [
        (
            "unknown_capacity",
            "run(capacity Length):\n    values ListResult(Byte): List.with_capacity(Byte, capacity)\n    when values is\n        List(items) then items.release\n        AllocFailure(failure) then failure.release\n    return\n",
            Some("type-parameter-refinement-unproven"),
        ),
        (
            "excessive_count",
            "run:\n    capacity: (max_count + 1) as Length\n    values ListResult(Byte): List.with_capacity(Byte, capacity)\n    when values is\n        List(items) then items.release\n        AllocFailure(failure) then failure.release\n    return\n",
            Some("type-parameter-refinement-failed"),
        ),
        (
            "excessive_bytes",
            "run:\n    capacity: max_count as Length\n    values ListResult(Word): List.with_capacity(Word, capacity)\n    when values is\n        List(items) then items.release\n        AllocFailure(failure) then failure.release\n    return\n",
            Some("type-parameter-refinement-failed"),
        ),
        (
            "unknown_growth",
            "run(values mut List(Byte), additional Length):\n    attempt ListReserveResult: values.reserve(additional)\n    return\n",
            Some("type-parameter-refinement-unproven"),
        ),
        (
            "guarded_capacity",
            "run(capacity Length):\n    when capacity <= max_count then\n        values ListResult(Byte): List.with_capacity(Byte, capacity)\n        when values is\n            List(items) then items.release\n            AllocFailure(failure) then failure.release\n    return\n",
            None,
        ),
        (
            "guarded_growth",
            "run(values mut List(Byte), additional Length):\n    when additional <= max_count - values.length.as_num then\n        attempt ListReserveResult: values.reserve(additional)\n    return\n",
            None,
        ),
        (
            "zero_sized_capacity",
            "run:\n    values ListResult(Empty): List.with_capacity(Empty, max_count as Length)\n    when values is\n        List(items) then items.release\n        AllocFailure(failure) then failure.release\n    return\n",
            None,
        ),
    ] {
        let package = TempPackage::new(&format!("list_capacity_{name}"));
        write_heap_fixture(&package);
        package.write(
            "collection/List.gin",
            include_str!("../../../modules/core/collection/List.gin"),
        );
        let source = format!(
            "use core.collection List, ListResult, ListReserveResult\nuse core.alloc max_count\nuse core.slice Byte, Length\nWord is in 0 to 18446744073709551615\nWord has release(self): return\nEmpty has release(self): return\n{body}"
        );
        let input = package.write("main.gin", &source);
        let mut driver = CompilerDriver::default();
        let parsed = driver
            .parse(SourcePackage::new(
                &input,
                vec![SourceFile::new(&input, source)],
            ))
            .unwrap();
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&host_target_triple()).unwrap(),
        );
        let checked =
            driver.check(CompilePackage::new(parsed, target).with_import_resolution(true));
        for file in checked.files() {
            assert!(
                file.front_end_diagnostics().is_empty(),
                "{name}: {:?}",
                file.front_end_diagnostics()
            );
            let flaws: Vec<_> = file
                .typecheck_diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.category == diagnostic::Category::Flaw)
                .collect();
            if file.path() == input.canonicalize().unwrap()
                && let Some(expected) = expected
            {
                assert!(
                    flaws.iter().any(|flaw| flaw.code.slug() == expected),
                    "{name}: {flaws:?}"
                );
                assert!(
                    flaws.iter().all(|flaw| flaw.code.slug() == expected),
                    "{name}: {flaws:?}"
                );
            } else {
                assert!(flaws.is_empty(), "{name}: {:?}: {flaws:?}", file.path());
            }
        }
    }
}

#[test]
fn compatible_allocator_handles_reclaim_storage() {
    let package = TempPackage::new("compatible_allocator_handles");
    write_heap_fixture(&package);
    let input = package.write(
        "main.gin",
        r#"use core.alloc Allocation, Allocator, AllocError, AllocationResult, Count, GlobalAllocator
use core.slice Byte
unwrap(attempt AllocationResult(Byte)) Allocation(Byte):
    return when attempt is
        Allocation(storage) then storage
        AllocError(error) then failed(error)
failed(error AllocError) Allocation(Byte): return failed(error)
DelegatingAllocator has Allocator
    inner GlobalAllocator
    reserve(self mut, capacity Count(x)) AllocationResult(x): return self.inner.reserve(capacity)
    reclaim(self mut, storage Allocation(x) and storage.initialized_count = 0):
        self.inner.reclaim(storage)
        return
    release(self):
        DelegatingAllocator(inner) := self
        inner.release
        return
#entry
main:
    original: GlobalAllocator()
    compatible: DelegatingAllocator(inner: GlobalAllocator())
    storage: unwrap(original.reserve(1 as Count(Byte)))
    when storage.initialized_count.as_num > 0 or storage.capacity_count.as_num < 1 then
        storage.release(original)
        compatible.release
        original.release
        return 1
    storage.initialize(0 as Count(Byte), 7 as Byte)
    value: storage.take_last
    value.release
    storage.reclaim(compatible)
    compatible.release
    original.release
    return 0
"#,
    );
    compile_and_run(&package, &input);
}

#[test]
fn allocator_implementations_must_prove_success_metadata_and_layout() {
    for (name, capacity, initialized, address) in [
        ("too_small", 0, 0, 4096_u64),
        ("nonempty", 4, 1, 4096),
        ("null", 4, 0, 0),
        ("misaligned", 4, 0, 1),
        ("upper_misaligned", 4, 0, u64::MAX),
    ] {
        let package = TempPackage::new(&format!("reserve_contract_{name}"));
        write_heap_fixture(&package);
        package.write(
            "alloc/allocator.gin",
            &format!(
                "use core.alloc GlobalAllocator\n{}\nBrokenAllocator has Allocator\n    reserve(self mut, requested Count(x)) AllocationResult(x):\n        requested.release\n        return Allocation(address: {address} as @x, capacity: {capacity} as Count(x), initialized: {initialized} as Count(x))\n    reclaim(self mut, storage Allocation(x) and storage.initialized_count = 0):\n        global: GlobalAllocator()\n        global.reclaim(storage)\n        global.release\n        return\n    release(self): return\n",
                include_str!("../../../modules/core/alloc/allocator.gin")
            ),
        );
        let input = package.write(
            "main.gin",
            "use core.alloc BrokenAllocator, Count\nWord is in 0 to 18446744073709551615\nWord has release(self): return\n#entry\nmain:\n    allocator: BrokenAllocator()\n    attempt: allocator.reserve(4 as Count(Word))\n    attempt.reclaim(allocator)\n    allocator.release\n    return 0\n",
        );
        let mut driver = CompilerDriver::default();
        let parsed = driver
            .parse(SourcePackage::new(
                &input,
                vec![SourceFile::new(
                    &input,
                    std::fs::read_to_string(&input).unwrap(),
                )],
            ))
            .unwrap();
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&host_target_triple()).unwrap(),
        );
        let checked =
            driver.check(CompilePackage::new(parsed, target).with_import_resolution(true));
        let front_end: Vec<_> = checked
            .files()
            .iter()
            .flat_map(|file| file.front_end_diagnostics())
            .collect();
        assert!(front_end.is_empty(), "{name}: {front_end:?}");
        let flaws: Vec<_> = checked
            .files()
            .iter()
            .flat_map(|file| file.typecheck_diagnostics())
            .collect();
        assert!(
            flaws.iter().any(|flaw| matches!(
                flaw.code.slug().as_str(),
                "type-postcondition-unproven" | "type-postcondition-failed"
            )),
            "{name}: {flaws:?}"
        );
    }
}

#[test]
fn initialized_regions_replace_transfer_and_release_values_once() {
    let package = TempPackage::new("initialized_regions");
    write_heap_fixture(&package);
    let input = package.write(
        "main.gin",
        include_str!("../../../crates/test-fixtures/fixtures/box_storage_initialized_regions.gin"),
    );
    compile_and_run(&package, &input);
}

#[test]
fn list_and_string_use_initialized_regions_for_growth_copy_and_append() {
    let package = TempPackage::new("managed_collections");
    write_heap_fixture(&package);
    package.write(
        "collection/List.gin",
        include_str!("../../../modules/core/collection/List.gin"),
    );
    package.write(
        "string/string.gin",
        include_str!("../../../modules/core/string/string.gin"),
    );
    let input = package.write(
        "main.gin",
        include_str!("../../../crates/test-fixtures/fixtures/box_storage_growth_copy_append.gin"),
    );
    compile_and_run(&package, &input);
}

#[test]
fn container_cleanup_releases_values_before_storage_and_allocator() {
    let package = TempPackage::new("terminal_container_order");
    write_heap_fixture(&package);
    package.write(
        "collection/List.gin",
        include_str!("../../../modules/core/collection/List.gin"),
    );
    package.write(
        "string/string.gin",
        include_str!("../../../modules/core/string/string.gin"),
    );
    let input = package.write(
        "main.gin",
        include_str!("../../../crates/test-fixtures/fixtures/box_storage_container_cleanup.gin"),
    );
    compile_and_run(&package, &input);
}

fn compile_and_run(package: &TempPackage, input: &std::path::Path) {
    compile_with_status(package, input, 0);
}

fn compile_with_status(package: &TempPackage, input: &std::path::Path, expected: i32) {
    let input = input.canonicalize().unwrap();
    assert_clean_lowering(&input);
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("program-{profile:?}"));
        compile_exe_checked(input.to_path_buf(), &executable, profile);
        let output = Command::new(&executable).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{profile:?}: {output:?}"
        );
    }
}

fn assert_clean_lowering(input: &std::path::Path) {
    let mut driver = CompilerDriver::default();
    let source = std::fs::read_to_string(input).unwrap();
    let parsed = driver
        .parse(SourcePackage::new(
            input,
            vec![SourceFile::new(input, source)],
        ))
        .unwrap();
    let target =
        flask::CompileTarget::Concrete(flask::TargetTriple::parse(&host_target_triple()).unwrap());
    let checked = driver.check(CompilePackage::new(parsed, target).with_import_resolution(true));
    let flaws: Vec<_> = checked
        .files()
        .iter()
        .flat_map(|file| {
            file.front_end_diagnostics()
                .iter()
                .chain(file.typecheck_diagnostics())
                .filter(|diagnostic| diagnostic.category == diagnostic::Category::Flaw)
                .map(|diagnostic| (file.path(), diagnostic))
        })
        .collect();
    assert!(flaws.is_empty(), "{flaws:?}");
    let context = codegen::NativeCompiler::create_context();
    let lowered = driver.lower_package(&context, &checked);
    let diagnostics: Vec<_> = lowered
        .diagnostics
        .iter()
        .flat_map(|file| &file.diagnostics)
        .collect();
    assert!(lowered.module.is_some(), "{diagnostics:?}");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn fallible_list_copy_and_growth_preserve_initialized_values() {
    let package = TempPackage::new("fallible_list_success");
    write_heap_fixture(&package);
    package.write(
        "collection/List.gin",
        include_str!("../../../modules/core/collection/List.gin"),
    );
    let input = package.write(
        "main.gin",
        r#"use core.collection List
use core.alloc AllocError, AllocFailure, GlobalAllocator, max_count
use core.ops CopyResult, Copied
use core.slice Byte
ResultCode is in 0 to 4
ResultCode has release(self): return
check_copy(values own List(Byte), duplicate own List(Byte) and duplicate.length = 2) ResultCode:
    first is in Byte: Byte.as_num(duplicate.borrow(0))
    second is in Byte: Byte.as_num(duplicate.borrow(1))
    duplicate.release
    values.release
    return when first = 7 and second = 9 then 0 else 1
exercise(values own List(Byte) and values.length = 0 and values.capacity >= 4) ResultCode:
    values.push_reserved(7 as Byte)
    values.push_reserved(9 as Byte)
    growth: values.reserve(10)
    when growth is AllocError(error) then
        values.release
        return 4
    attempt: values.copy
    return when attempt is
        Copied(copied) then check_copy(values, copied.take)
        AllocError(error) then
            values.release
            return 2
#entry
main:
    attempt: List.with_capacity(Byte, 4)
    return when attempt is
        List(values) then exercise(values)
        AllocFailure(failure) then
            failure.release
            return 3
"#,
    );
    compile_and_run(&package, &input);
}

#[test]
fn fallible_list_push_returns_the_value_and_keeps_the_original_storage() {
    let package = TempPackage::new("fallible_list_failure");
    write_heap_fixture(&package);
    package.write(
        "collection/List.gin",
        include_str!("../../../modules/core/collection/List.gin"),
    );
    let input = package.write(
        "main.gin",
        include_str!("../../../crates/test-fixtures/fixtures/box_storage_fallible_list_push.gin"),
    );
    compile_and_run(&package, &input);
}

fn write_heap_fixture(package: &TempPackage) {
    package.write(
        "string/utf8/utf8.gin",
        include_str!("../../../modules/core/string/utf8/utf8.gin"),
    );
    package.write_flask("core");
    for (path, source) in [
        (
            "alloc/box.gin",
            include_str!("../../../modules/core/alloc/box.gin"),
        ),
        (
            "alloc/error.gin",
            include_str!("../../../modules/core/alloc/error.gin"),
        ),
        (
            "alloc/address.gin",
            include_str!("../../../modules/core/alloc/address.gin"),
        ),
        (
            "alloc/allocator.gin",
            include_str!("../../../modules/core/alloc/allocator.gin"),
        ),
        (
            "alloc/count.gin",
            include_str!("../../../modules/core/alloc/count.gin"),
        ),
        (
            "alloc/global.gin",
            include_str!("../../../modules/core/alloc/global.gin"),
        ),
        (
            "ops/copy.gin",
            include_str!("../../../modules/core/ops/copy.gin"),
        ),
        (
            "ops/release.gin",
            include_str!("../../../modules/core/ops/release.gin"),
        ),
        (
            "ops/numeric.gin",
            include_str!("../../../modules/core/ops/numeric.gin"),
        ),
        (
            "range/range.gin",
            include_str!("../../../modules/core/range/range.gin"),
        ),
    ] {
        package.write(path, source);
    }
    package.write(
        "slice/slice.gin",
        include_str!("../../../modules/core/slice/slice.gin"),
    );
    package.write(
        "alloc/system/system.gin",
        &heap_backend_source(&host_target_triple()),
    );
}

fn heap_backend_source(target: &str) -> String {
    let (reserve, release, architecture, size, pointer, length, reserve_clobbers, release_clobbers) =
        match target {
            "aarch64-apple-darwin" => (
                "Arm64DarwinReserve",
                "Arm64DarwinRelease",
                "aarch64",
                9,
                9,
                10,
                vec![1, 2, 3, 4, 5, 16],
                vec![1, 16],
            ),
            "aarch64-unknown-linux-gnu" => (
                "Arm64LinuxReserve",
                "Arm64LinuxRelease",
                "aarch64",
                9,
                9,
                10,
                vec![1, 2, 3, 4, 5, 8],
                vec![1, 8],
            ),
            "x86_64-unknown-linux-gnu" => (
                "X86LinuxReserve",
                "X86LinuxRelease",
                "x86_64",
                6,
                7,
                6,
                vec![7, 2, 10, 8, 9, 1, 11],
                vec![1, 11],
            ),
            _ => panic!("unsupported heap fixture target: {target}"),
        };
    let register = |index| {
        format!(r#"[architecture: "{architecture}", bank: "general", index: {index}, width: 64]"#)
    };
    let flags = format!(r#"[architecture: "{architecture}", bank: "flags", index: 0, width: 4]"#);
    let clobbers = |indices: Vec<u32>| {
        let mut values: Vec<_> = indices.into_iter().map(register).collect();
        values.push(flags.clone());
        values.join(", ")
    };
    let (reserve_count, reserve_bytes) = platform_instructions(reserve);
    let (release_count, release_bytes) = platform_instructions(release);
    let (exit_name, exit_register, exit_clobbers) = match target {
        "aarch64-apple-darwin" => ("Arm64DarwinExit", 9, vec![0, 1, 16]),
        "aarch64-unknown-linux-gnu" => ("Arm64LinuxExit", 9, vec![0, 8]),
        "x86_64-unknown-linux-gnu" => ("X86LinuxExit", 7, vec![0, 1, 11]),
        _ => unreachable!(),
    };
    let (exit_count, exit_bytes) = platform_instructions(exit_name);
    let exit_register = register(exit_register);
    let exit_clobbers = clobbers(exit_clobbers);
    let size = register(size);
    let pointer = register(pointer);
    let length = register(length);
    let output = register(0);
    let reserve_clobbers = clobbers(reserve_clobbers);
    let release_clobbers = clobbers(release_clobbers);
    include_str!("../../../crates/test-fixtures/fixtures/box_storage_heap_backend.gin")
        .replace("{exit_count}", &exit_count.to_string())
        .replace("{exit_bytes}", &exit_bytes.to_string())
        .replace("{reserve_count}", &reserve_count.to_string())
        .replace("{reserve_bytes}", &reserve_bytes.to_string())
        .replace("{release_count}", &release_count.to_string())
        .replace("{release_bytes}", &release_bytes.to_string())
        .replace("{exit_register}", &exit_register.to_string())
        .replace("{exit_clobbers}", &exit_clobbers.to_string())
        .replace("{size}", &size.to_string())
        .replace("{output}", &output.to_string())
        .replace("{reserve_clobbers}", &reserve_clobbers.to_string())
        .replace("{pointer}", &pointer.to_string())
        .replace("{length}", &length.to_string())
        .replace("{release_clobbers}", &release_clobbers.to_string())
}

fn platform_instructions(name: &str) -> (usize, &str) {
    let source = include_str!("../../../modules/core/alloc/system/system.gin");
    let declaration = source.split(&format!("{name} has\n")).nth(1).unwrap();
    let function = declaration.lines().next().unwrap();
    let bytes = function.split_once(": ").unwrap().1;
    (bytes.matches("as MachineByte").count(), bytes)
}

#[test]
fn failed_deep_copy_releases_partial_values_and_preserves_the_source() {
    let package = TempPackage::new("partial_copy_failure");
    write_heap_fixture(&package);
    package.write(
        "collection/List.gin",
        include_str!("../../../modules/core/collection/List.gin"),
    );
    let input = package.write(
        "main.gin",
        include_str!("../../../crates/test-fixtures/fixtures/box_storage_failed_deep_copy.gin"),
    );
    compile_and_run(&package, &input);
}
