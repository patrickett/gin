use flask::CompileTarget;
use ginfmt::help;
use test_fixtures::TempPackage;

const CODE: &str = "fmt-unused-generic-type-name";

fn fixture(source: &str) -> (TempPackage, std::path::PathBuf) {
    let package = TempPackage::new("generic_names");
    package.write_flask("generic_names");
    let path = package.write("main.gin", source);
    (package, path)
}

#[test]
fn removes_single_use_types_and_preserves_parameter_conventions() {
    for (parameter, expected) in [
        ("a alloc", "a"),
        ("a mut alloc", "a mut"),
        ("a ref alloc", "a ref"),
        ("a own alloc", "a own"),
        ("a ref{storage} alloc", "a ref{storage}"),
    ] {
        let (_package, path) = fixture(&format!("consume({parameter}): return a.release\n"));
        let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
        let hint = hints
            .iter()
            .find(|hint| hint.diagnostic.code.slug() == CODE)
            .unwrap_or_else(|| panic!("{parameter}: {hints:?}"));
        assert_eq!(hint.diagnostic.category, diagnostic::Category::Help);
        assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("consume({expected}): return a.release\n")
        );
        assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
    }
}

#[test]
fn keeps_type_relationships_body_uses_and_explicit_type_parameters() {
    for source in [
        "same(a t, b t): return a.release\n",
        "identity(a t) t: return a\n",
        "identity(a t) result Box(t): return a\n",
        "width(a t): return #size(t)\n",
        "convert(a t): return 0 as t\n",
        "consume(a t): return t.release(a)\n",
        "consume(t, a t): return a.release\n",
        "consume(a Box(t)): return a.release\n",
        "consume(a @t):\n    return\n",
        "consume(a t, b ref has Convert(t)):\n    return\n",
        "Holder(t) has\n    consume(self, a t): return a.release\n",
        "consume(a Concrete): return a.release\n",
        "outer(a t):\n    inner(b t): return b.release\n    return a\n",
    ] {
        let (_package, path) = fixture(source);
        let hints = help::suggestions(&path, CompileTarget::Library)
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert!(
            !hints.iter().any(|hint| hint.diagnostic.code.slug() == CODE),
            "{source}: {hints:?}"
        );
    }
}

#[test]
fn methods_and_independent_functions_get_separate_fixes() {
    let source = "Holder has\n    consume(self, a t): return a.release\n    inspect(self ref, b ref t)\nfirst(a t): return a.release\nsecond(b t): return b.release\n";
    let (_package, path) = fixture(source);
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    assert_eq!(
        hints
            .iter()
            .filter(|hint| hint.diagnostic.code.slug() == CODE)
            .count(),
        4,
        "{hints:?}"
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 4);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("consume(self, a)"), "{fixed}");
    assert!(fixed.contains("inspect(self ref, b ref)"), "{fixed}");
}

#[test]
fn ignores_comments_and_strings_but_preserves_comments_in_the_annotation() {
    let (_package, path) =
        fixture("consume(a mut t):\n    -- t\n    bytes: 't'\n    return a.release\n");
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("consume(a mut):"), "{fixed}");
    assert!(fixed.contains("-- t\n    bytes: 't'"), "{fixed}");
    let (_package, path) = fixture("consume(\n    a mut -- keep\n        t\n): return a.release\n");
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    assert!(
        !hints.iter().any(|hint| hint.diagnostic.code.slug() == CODE),
        "{hints:?}"
    );
}

#[test]
fn fix_preserves_calls_and_mutable_borrowing() {
    let source = "Store has\n    touch(self mut):\n        return\n    release(self):\n        return\napply(a mut store): return a.touch\nrun:\n    s: Store()\n    apply(s)\n    s.release\n    return\n";
    let (_package, path) = fixture(source);
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("apply(a mut): return a.touch"), "{fixed}");
}
