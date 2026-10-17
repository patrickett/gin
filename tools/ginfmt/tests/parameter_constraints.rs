use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

#[test]
fn formatting_preserves_parameter_constraints_and_conventions() {
    for source in [
        "reclaim(self, a mut has Allocator): return a.reclaim(self)\n",
        "inspect(a ref has Convert(Word) and Copy): return a.read\n",
        "Allocation(x) has\n    reclaim(self, a mut has Allocator): return a.reclaim(self)\n",
    ] {
        let formatted = ginfmt::format(source);
        assert!(
            formatted.contains(if source.contains("Convert") {
                "a ref has Convert(Word) and Copy"
            } else {
                "a mut has Allocator"
            }),
            "{formatted}"
        );
        let parsed = formatted.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        assert_roundtrip(&formatted);
    }
}

#[test]
fn formatting_preserves_proposition_annotations_and_guards() {
    let source = "less(value is in 0 to 255, check is value < 16) is value < 16: return check\nrun(value is in 0 to 255):\n    small is value < 16: value < 16\n    return when small then 1 else 0\n";
    let formatted = ginfmt::format(source);
    assert!(formatted.contains("check is value < 16"), "{formatted}");
    assert!(formatted.contains("small is value < 16"), "{formatted}");
    let parsed = formatted.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert_roundtrip(&formatted);
}

#[test]
fn formatting_preserves_success_payload_contracts() {
    for source in [
        "reserve(c Count) storage Allocation(x) thus storage.initialized_count = 0 and storage.capacity_count >= c or AllocError: return OutOfMemory\n",
        "Allocator has\n    reserve(self mut, c Count(x)) storage Allocation(x) thus storage.initialized_count = 0 and storage.capacity_count >= c or AllocError\n",
    ] {
        let formatted = ginfmt::format(source);
        assert!(
            formatted.contains("storage Allocation(x) thus")
                && formatted.contains("storage.initialized_count = 0"),
            "{formatted}"
        );
        assert!(
            formatted.contains("storage.capacity_count >= c")
                && formatted.contains("or AllocError"),
            "{formatted}"
        );
        let parsed = formatted.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{:?}\n{formatted}",
            parsed.symptoms
        );
        assert_roundtrip(&formatted);
    }
}
