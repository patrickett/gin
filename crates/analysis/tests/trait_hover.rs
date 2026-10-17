use analysis::PackageCache;
use test_fixtures::TempPackage;

#[test]
fn required_traits_show_type_names_and_trait_docs() {
    let pkg = TempPackage::new("required_trait_hover");
    pkg.write_flask("actions");
    pkg.write(
        "ops/duplicate.gin",
        "--- Produces a duplicate while retaining the source.\nDuplicate has\n    --- Duplicate this value.\n    duplicate(self ref) Self: return self\n",
    );
    pkg.write(
        "ops/dispose.gin",
        "--- Disposes of resources owned by a value.\nDispose has\n    --- Consume this value.\n    dispose(self):\n        return\n",
    );
    let source = "use actions.ops Duplicate, Dispose\nDocument has Duplicate and Dispose\n";
    let path = pkg.write("text/document.gin", source);
    let engine = PackageCache::for_test_with_file(path.clone());

    for (name, docs) in [
        (
            "Duplicate",
            "Produces a duplicate while retaining the source.",
        ),
        ("Dispose", "Disposes of resources owned by a value."),
    ] {
        let expected = format!("```gin\n{name}\n```\n---\n\n{docs}");
        for (byte, _) in source.match_indices(name) {
            let hover = engine.hover(&path, byte as u32).expect(name);
            assert_eq!(hover.markdown, expected, "{name} at {byte}");
            assert_eq!(hover.byte_range, Some(byte..byte + name.len()));
        }
    }
}
