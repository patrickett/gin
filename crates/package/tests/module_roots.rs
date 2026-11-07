//! A run may begin at any module of a package, or at every module of a library that has
//! nothing at its root.
use std::fs;
use std::path::Path;

use package::{GraphError, GraphLimits, LoadError, Package, PackageError};
use typecheck::Limits;

const MANIFEST: &str = "#settings\nmodule: \"lib\"\nversion: 0.1.0\ntarget: \"library\"\n";
const ONE: &str = "one() in 0 to 1: return 1\n";

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn package(files: &[(&str, &str)]) -> (tempfile::TempDir, Package) {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "mod.flask", MANIFEST);
    for (path, text) in files {
        write(root.path(), path, text);
    }
    let package = Package::open(root.path()).unwrap();
    (root, package)
}

fn names(graph: &package::PackageGraph) -> Vec<&str> {
    graph.modules.iter().map(|module| module.name.as_str()).collect()
}

fn segments(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

#[test]
fn modules_are_found_in_sorted_order_skipping_target_and_hidden_folders() {
    let (_root, package) = package(&[
        ("b/x.gin", ONE),
        ("a/y.gin", ONE),
        ("a/deeper/z.gin", ONE),
        ("target/debug/skip.gin", ONE),
        (".hidden/skip.gin", ONE),
        ("empty/readme.txt", "not source"),
    ]);
    let found = package.module_segments(64).unwrap();
    assert_eq!(found, [segments(&["a"]), segments(&["a", "deeper"]), segments(&["b"])]);
    assert!(!package.has_sources_at(&[]));
    assert!(package.has_sources_at(&segments(&["a"])));
    assert!(!package.has_sources_at(&segments(&["empty"])));
    assert!(!package.has_sources_at(&segments(&["missing"])));
}

#[test]
fn the_number_of_modules_is_bounded() {
    let (_root, package) = package(&[("a/x.gin", ONE), ("b/x.gin", ONE), ("c/x.gin", ONE)]);
    assert!(matches!(
        package.module_segments(2),
        Err(GraphError::TooManyModules { max: 2 })
    ));
    assert_eq!(package.module_segments(3).unwrap().len(), 3);
}

#[test]
fn a_graph_can_begin_at_a_module_below_the_root() {
    let (_root, package) = package(&[
        ("main.gin", "use lib.util one\nroot() in 0 to 1: return one()\n"),
        ("util/util.gin", ONE),
        ("app/app.gin", "use lib.util one\nuse lib.deep one as deeper\nentry() in 0 to 1: return one()\n"),
        ("deep/deep.gin", ONE),
    ]);
    let graph = package.load_graph_at(&segments(&["app"]), GraphLimits::default()).unwrap();
    assert_eq!(names(&graph), ["lib.util", "lib.deep", "lib.app"]);
    assert_eq!(graph.root, Some(2));
    // The root module of the package is not part of this run.
    assert!(!names(&graph).contains(&"lib"));
    let default = package.load_graph(GraphLimits::default()).unwrap();
    assert_eq!(default.root, Some(default.modules.len() - 1));
    assert_eq!(default.modules.last().unwrap().name, "lib");
}

#[test]
fn a_missing_module_is_named() {
    let (_root, package) = package(&[("main.gin", ONE)]);
    let error = package.load_graph_at(&segments(&["nowhere"]), GraphLimits::default());
    assert!(
        matches!(error, Err(GraphError::Load(LoadError::ModuleNotFound { .. }))),
        "{error:?}"
    );
}

#[test]
fn a_library_with_an_empty_root_is_checked_as_all_its_modules() {
    let (_root, package) = package(&[
        ("a/a.gin", "use lib.b one\nfirst() in 0 to 1: return one()\n"),
        ("b/b.gin", ONE),
        ("c/c.gin", ONE),
    ]);
    let graph = package.load_graph_all(GraphLimits::default()).unwrap();
    // Dependency order, and a module reached by an import is loaded once.
    assert_eq!(names(&graph), ["lib.b", "lib.a", "lib.c"]);
    assert_eq!(graph.root, None);
    let checked = package.check_all(GraphLimits::default(), Limits::default()).unwrap();
    assert_eq!(checked.capsules.len(), 3);
    assert_eq!(checked.entry(), None);
}

#[test]
fn a_package_with_no_source_at_all_has_nothing_to_check() {
    let (_root, package) = package(&[("notes.txt", "x")]);
    let graph = package.load_graph_all(GraphLimits::default()).unwrap();
    assert!(graph.modules.is_empty() && graph.root.is_none());
    let checked = package.check_all(GraphLimits::default(), Limits::default()).unwrap();
    assert!(checked.capsules.is_empty());
    assert_eq!(checked.entry(), None);
}

#[test]
fn a_failure_in_any_module_of_a_library_is_reported() {
    let (_root, package) = package(&[("a/a.gin", ONE), ("b/b.gin", "bad() in 0 to 1: return 5\n")]);
    let error = package.check_all(GraphLimits::default(), Limits::default());
    assert!(
        matches!(&error, Err(PackageError::Check { module, .. }) if module == "lib.b"),
        "{error:?}"
    );
}

#[test]
fn only_the_selected_root_may_mark_an_entry() {
    let entry = "#entry\nmain() in 0 to 1: return 1\n";
    let (_root, package) = package(&[("app/app.gin", entry), ("other/other.gin", ONE)]);
    // Selected as the root, the module's entry is the program's.
    let checked = package
        .check_at(&segments(&["app"]), GraphLimits::default(), Limits::default())
        .unwrap();
    assert!(checked.entry().is_some());
    // Reached by an import of another root, or among all modules, it is not.
    let error = package.check_all(GraphLimits::default(), Limits::default());
    assert!(matches!(error, Err(PackageError::EntryOutsideRoot { .. })), "{error:?}");
}
