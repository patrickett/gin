use analysis::PackageCache;
use parser::query::SourceParseExt;
use std::path::PathBuf;

#[test]
fn self_in_member_bodies_has_the_same_hover_as_its_signature() {
    let source = "Byte is in 0 to 255\n--- A stored value.\nCell has\n    value Byte\n    new(value Byte) Self:\n        stored: value\n        return Self(value: stored)\n    copy(self ref) Self: return Self(value: self.value)\n\n--- A separate type.\nOther has\n    value Byte\n    new(value Byte) Self: return Self(value)\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let path = PathBuf::from("/tmp/member_self_hover.gin");
    let engine = PackageCache::for_test_with_file(path.clone());
    engine.set_contents(&path, source.to_string());

    for (section, owner) in [
        (source.split("--- A separate").next().unwrap(), "Cell"),
        (source.split("--- A separate").nth(1).unwrap(), "Other"),
    ] {
        let signature = section.find("Self").unwrap();
        let section_start = source.find(section).unwrap();
        let expected = engine
            .hover(&path, (section_start + signature) as u32)
            .expect("Self signature hover")
            .markdown;
        assert!(expected.contains(&format!("{owner} has")), "{expected}");
        for (byte, _) in section.match_indices("Self") {
            let byte = section_start + byte;
            let hover = engine.hover(&path, byte as u32).expect("Self body hover");
            assert_eq!(hover.markdown, expected, "{owner} at {byte}");
            assert_eq!(hover.byte_range, Some(byte..byte + 4));
        }
    }
}
