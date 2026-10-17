use analysis::PackageCache;
use parser::query::SourceParseExt;
use test_fixtures::TempPackage;

#[test]
fn specialized_member_hover_uses_the_resolved_receivers_source_signature_and_docs() {
    let pkg = TempPackage::new("specialized_member_hover");
    pkg.write_flask("sample");
    pkg.write("collection/container.gin", "Container(element) has\n    value element\n    --- Appends another container.\n    append(self mut, other own Self):\n        return\n");
    let source = "use sample.collection Container\nByte is in 0 to 255\nWrapper has\n    value Container(Byte)\n    --- Appends another wrapper.\n    append(self mut, other own Self):\n        return\n    combine(left own Self, right own Self):\n        Wrapper(value: first) := left\n        Wrapper(value: second) := right\n        first.append(second)\n        return\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let path = pkg.write("text/wrapper.gin", source);
    let engine = PackageCache::for_test_with_file(path.clone());
    let byte = source.find("append(second)").unwrap();
    let hover = engine.hover(&path, byte as u32).expect("append hover");
    assert!(
        hover.markdown.contains("append(self mut, other own Self)"),
        "{}",
        hover.markdown
    );
    assert!(
        hover.markdown.contains("Appends another container."),
        "{}",
        hover.markdown
    );
    assert!(
        !hover.markdown.contains("Appends another wrapper."),
        "{}",
        hover.markdown
    );
    assert!(!hover.markdown.contains('$'), "{}", hover.markdown);
    assert_eq!(hover.byte_range, Some(byte..byte + "append".len()));
}
