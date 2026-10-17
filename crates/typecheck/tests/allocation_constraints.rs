mod support;
use parser::query::SourceParseExt;
use support::{check, flaw_codes};

#[test]
fn generic_owned_storage_initialization_and_cleanup_are_proven() {
    let source = format!(
        "{}\nByte is in 0 to 255\nByte has release(self): return\n#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has Numeric\n    release(self): return\nmax_count := 255\nCopied(element) has\n    value element\n    take(self) element: return self.value\n    release(self): return\nreplace_count(unit, previous Count(unit), next Count(unit)) Count(unit): return next\n{}\n",
        include_str!("../../../modules/core/ops/numeric.gin"),
        include_str!("../../../modules/core/alloc/allocator.gin")
            .split("AllocError is")
            .next()
            .unwrap()
            .replace("use core.alloc AllocError, AllocFailure, Count, StorageAddress, replace_count, max_count\n", "")
            .replace("use core.ops Copied\n", "")
            .replace("use core.slice Length, SliceMutView, SliceView\n", "")
            .replace("use core.alloc.system exit_process\n", "")
    );
    let start = source.find("    borrow(self").unwrap();
    let end = source.find("    take_last(self").unwrap();
    let mut source = format!("{}{}", &source[..start], &source[end..]);
    let start = source.find("    copy_into(self").unwrap();
    let end = source.find("    release_values(self").unwrap();
    source.replace_range(start..end, "");
    let start = source.find("    replace(self").unwrap();
    let end = source.find("    move_into(self").unwrap();
    source.replace_range(start..end, "");
    source.truncate(source.find("    view(self").unwrap());
    source.push_str("    reclaim(self):\n        Allocation(address, capacity, initialized) := self\n        pointer: address as @x\n        return\nuse_storage(storage mut Allocation(Byte) and storage.initialized_count = 0 and storage.capacity_count > 0):\n    storage.initialize(0 as Count(Byte), 1 as Byte)\n    storage.release_values\n    return thus storage.initialized_count = 0\ntransfer(source mut Allocation(Byte) and source.initialized_count = 1, destination mut Allocation(Byte) and destination.initialized_count = 0 and destination.capacity_count >= source.initialized_count):\n    source.move_into(destination)\n    value: destination.take_last\n    value.release\n    return thus source.initialized_count = 0 and destination.initialized_count = 0\n");
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = check(&source);
    assert_eq!(
        flaw_codes(&typed),
        Vec::<&str>::new(),
        "{:?}\n{source}",
        typed.all_flaws()
    );
}

#[test]
fn allocation_capacity_proves_byte_size_after_destructuring() {
    let source = format!(
        "{}\n#phantom(unit)\nCount(unit) is in 0 to 9223372036854775807\nCount(unit) has Numeric\n    release(self): return\nMemorySize is in 0 to 18446744073709551615\nMemorySize has release(self): return\nmax_count := 9223372036854775807\nAllocation(x) has\n    capacity Count(x) and capacity * #size(x) <= max_count\nbyte_size(a Allocation(x)) MemorySize:\n    Allocation(capacity) := a\n    length: capacity.to_num\n    width: #size(x)\n    return (length * width) as MemorySize\n",
        include_str!("../../../modules/core/ops/numeric.gin"),
    );
    support::check_no_flaws(&source);
}

#[test]
fn abstract_target_allocation_preserves_byte_size_proofs() {
    let counts = include_str!("../../../modules/core/alloc/count.gin")
        .replace("use core.slice Byte\n", "")
        .replace("use core.ops Numeric\n", "");
    let source = format!(
        "{}\nByte is in 0 to 255\nByte has release(self): return\n{counts}\nMemorySize is in 0 to 18446744073709551615\nMemorySize has release(self): return\nAllocation(x) has\n    capacity Count(x) and capacity * #size(x) <= max_count\nbyte_size(a Allocation(x)) MemorySize:\n    Allocation(capacity) := a\n    length: capacity.to_num\n    width: #size(x)\n    return (length * width) as MemorySize\nrequested_bytes(capacity is in Count(x)) MemorySize:\n    requested: capacity * #size(x)\n    when requested > max_count then return 0\n    return requested as MemorySize\n",
        include_str!("../../../modules/core/ops/numeric.gin"),
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast;
    assert!(typecheck::prepare_parse_ast(&mut ast, &flask::CompileTarget::Library).is_empty());
    let typed = typecheck::transform::transform(
        &ast,
        typecheck::FileId(0),
        &typecheck::transform::TransformCtx::with_package_compile_time_arc(std::sync::Arc::new(
            ast.clone(),
        )),
    );
    support::expect_no_flaws(&typed);
    let source = format!(
        "{source}\nunchecked_bytes(capacity is in Count(x)) MemorySize: return (capacity * #size(x)) as MemorySize
shadowed_limit(capacity is in Count(x), max_count in 0 to 340282366920938463463374607431768211455) MemorySize:
    requested: capacity * #size(x)
    when requested > max_count then return 0
    return requested as MemorySize
"
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast;
    assert!(typecheck::prepare_parse_ast(&mut ast, &flask::CompileTarget::Library).is_empty());
    let typed = typecheck::transform::transform(
        &ast,
        typecheck::FileId(0),
        &typecheck::transform::TransformCtx::with_package_compile_time_arc(std::sync::Arc::new(
            ast.clone(),
        )),
    );
    assert_eq!(
        flaw_codes(&typed)
            .iter()
            .filter(|code| **code == "type-integer-narrowing-failed")
            .count(),
        2,
        "{:?}",
        typed.all_flaws()
    );
}
