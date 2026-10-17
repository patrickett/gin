use analysis::package_cache::PackageEntry;
use analysis::{
    DocumentSnapshot, HoverContent, PackageCache, SemanticQueries, SymbolKind,
    format_hover_markdown,
};
use ast::source::SourceExt;
use test_fixtures::TempPackage;

fn fixture(source: &str) -> (TempPackage, PackageCache, std::path::PathBuf) {
    let package = TempPackage::new("semantic_queries");
    package.write_flask("semantic_queries");
    let path = package.write("main.gin", source).canonicalize().unwrap();
    let cache = PackageCache::for_test_with_file(path.clone());
    (package, cache, path)
}

fn queries<'a>(snapshot: &'a DocumentSnapshot, entry: &'a PackageEntry) -> SemanticQueries<'a> {
    let index = entry
        .file_paths
        .iter()
        .position(|path| *path == snapshot.path)
        .unwrap();
    SemanticQueries::new(
        &snapshot.source,
        &snapshot.parse.ast,
        &entry.typed_asts[index],
        &entry.package_index,
    )
}

#[test]
fn root_completions_use_typed_surface_symbols_and_unsaved_overloads() {
    let (_package, cache, path) = fixture("old: 1\n");
    let source = concat!(
        "-- café 🦀\nFirst is in 0 to 10\nSecond is in 0 to 10\n",
        "--- First overload.\nchoose(value First) First: return value\n",
        "choose(value Second) First: return 1\n\n",
    );
    cache.set_contents(&path, source.to_owned());
    let snapshot = cache.document_snapshot(&path).unwrap();
    let entry = cache
        .get_or_compute_package(std::slice::from_ref(&path))
        .unwrap();
    let engine = queries(&snapshot, &entry);
    let symbols = engine.scope();
    let overloads: Vec<_> = symbols
        .iter()
        .filter(|symbol| symbol.name == "choose")
        .collect();
    assert_eq!(overloads.len(), 2, "{symbols:?}");
    assert!(
        overloads
            .iter()
            .all(|symbol| symbol.kind == SymbolKind::Function)
    );
    assert!(
        overloads
            .iter()
            .all(|symbol| symbol.ty.as_ref().unwrap().format_for_hover() == "First")
    );
    assert!(
        overloads[0]
            .documentation
            .as_ref()
            .unwrap()
            .contains("First overload")
    );
    let candidates = cache.completions_at(&path, source.len() as u32);
    let functions: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.label == "choose")
        .collect();
    assert_eq!(functions.len(), 2, "{candidates:?}");
    assert!(
        functions
            .iter()
            .any(|candidate| candidate.detail.as_deref() == Some("choose(value First) First"))
    );
    assert!(
        functions
            .iter()
            .any(|candidate| candidate.detail.as_deref() == Some("choose(value Second) First"))
    );
    assert!(!candidates.iter().any(|candidate| candidate.label == "old"));
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.label.contains('(') || candidate.label.contains('$'))
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "old: 1\n");
}

#[test]
fn completions_use_semantic_signatures_through_noncanonical_paths() {
    let package = TempPackage::new("query_path_alias");
    package.write_flask("query_path_alias");
    let source = "Word is in 0 to 255\nread(value Word) Word: return value\n\n";
    let path = package.write("main.gin", source);
    let cache = PackageCache::for_test_with_file(path.clone());
    let candidates = cache.completions_at(&path, source.len() as u32);
    assert!(
        candidates.iter().any(|candidate| {
            candidate.label == "read"
                && candidate.detail.as_deref() == Some("read(value Word) Word")
        }),
        "{candidates:?}"
    );
}

#[test]
fn semantic_completions_keep_lexical_shadowing_and_compiler_queries() {
    let source = concat!(
        "Word is in 0 to 255\ncount Word: 7\n",
        "read(count Word) Word:\n    return count\n\n",
    );
    let (_package, cache, path) = fixture(source);
    let position = source.rfind("count").unwrap() + 2;
    let snapshot = cache.document_snapshot(&path).unwrap();
    let entry = cache
        .get_or_compute_package(std::slice::from_ref(&path))
        .unwrap();
    let engine = queries(&snapshot, &entry);
    let candidates = engine.completions(position);
    let shadowed: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.label == "count")
        .collect();
    assert_eq!(shadowed.len(), 1, "{candidates:?}");
    assert_eq!(shadowed[0].detail.as_deref(), Some("parameter"));
    let source = format!("{source}#to");
    cache.set_contents(&path, source.clone());
    let candidates = cache.completions_at(&path, source.len() as u32);
    assert!(
        candidates.iter().any(|candidate| candidate.label == "todo"),
        "{candidates:?}"
    );
}

#[test]
fn byte_hover_matches_the_existing_utf16_query_api() {
    for (source, needle) in [
        ("-- café 🦀\nmain:\n    x := 1\n    return 0\n", "x :="),
        ("main:\n    x: 1\n    return 0\n", "x: 1"),
    ] {
        let (_package, cache, path) = fixture(source);
        let snapshot = cache.document_snapshot(&path).unwrap();
        let entry = cache
            .get_or_compute_package(std::slice::from_ref(&path))
            .unwrap();
        let index = entry
            .file_paths
            .iter()
            .position(|file| *file == path)
            .unwrap();
        let byte = source.find(needle).unwrap();
        let (line, character) = source.byte_offset_to_position(byte);
        let existing = entry.typed_asts[index]
            .hover_at_with_package(source, line, character, Some(&entry.package_index))
            .unwrap();
        let expected = HoverContent {
            markdown: format_hover_markdown(&existing.markdown, existing.module_prefix.as_deref()),
            byte_range: source.word_byte_range(byte).map(|(start, end)| start..end),
        };
        let engine = queries(&snapshot, &entry);
        assert_eq!(engine.hover(byte), Some(expected.clone()));
        assert_eq!(cache.hover(&path, byte as u32), Some(expected));
        assert!(engine.hover(source.len() + 1).is_none());
    }
}

#[test]
fn querying_unimported_symbols_preserves_diagnostics_and_canonical_paths() {
    let package = TempPackage::new("query_visibility");
    package.write_flask("query_visibility");
    let happy = package.write("happy.gin", "Happy has value Bool\n");
    let source = "Bool is True or False\nBool has Happy\n";
    let path = package.write("bool.gin", source);
    let cache = PackageCache::for_test_with_file(happy.clone());
    cache.add_file(path.clone()).unwrap();
    let files = [happy, path.clone()];
    let before = cache.all_diagnostics(&files);
    assert!(
        cache
            .hover(&path, source.find("Happy").unwrap() as u32)
            .is_none()
    );
    assert_eq!(cache.all_diagnostics(&files), before);
    assert!(
        before[&path.canonicalize().unwrap()]
            .iter()
            .any(|diagnostic| {
                diagnostic.code.slug() == "type-unknown-symbol"
                    && diagnostic.arg("name") == Some("Happy")
            })
    );
}
