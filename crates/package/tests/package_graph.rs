//! Whole-package loading and checking: import mapping, ordering, and negative controls.
use std::fs;
use std::path::Path;

use package::{GraphError, GraphLimits, LoadError, Package, PackageError};
use typecheck::{Limits, SourceCheckError};

const MANIFEST: &str = "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n";
const UTIL: &str = "five() in 0 to 16: return 5\n";
const MAIN: &str = "use app.util five\nagain() in 0 to 16: return five()\n";

fn package(files: &[(&str, &str)]) -> (tempfile::TempDir, Package) {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "mod.flask", MANIFEST);
    for (path, text) in files {
        write(root.path(), path, text);
    }
    let package = Package::open(root.path()).expect("manifest opens");
    (root, package)
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn check(package: &Package) -> Result<package::CheckedPackage, PackageError> {
    package.check(GraphLimits::default(), Limits::default())
}

#[test]
fn a_two_module_package_checks_with_dependencies_first() {
    let (_root, package) = package(&[("main.gin", MAIN), ("util/five.gin", UTIL)]);
    let checked = check(&package).expect("toy package checks");
    let names: Vec<_> = checked
        .graph
        .modules
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(names, ["app.util", "app"]);
    assert_eq!(checked.graph.imports, [vec![], vec![0]]);
    assert_eq!(checked.capsules.len(), 2);
    let own: Vec<_> = checked.capsules[1]
        .checked()
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    assert_eq!(own, ["again"]);
}

#[test]
fn a_shared_dependency_is_loaded_once() {
    let (_root, package) = package(&[
        (
            "main.gin",
            "use app.a one\nuse app.b two\nuse app.util five\n",
        ),
        (
            "a/a.gin",
            "use app.util five\none() in 0 to 16: return five()\n",
        ),
        (
            "b/b.gin",
            "use app.util five\ntwo() in 0 to 16: return five()\n",
        ),
        ("util/five.gin", UTIL),
    ]);
    let graph = package.load_graph(GraphLimits::default()).unwrap();
    assert_eq!(graph.modules.len(), 4);
    assert_eq!(graph.modules[0].name, "app.util");
    assert_eq!(graph.modules[3].name, "app");
}

#[test]
fn a_missing_module_folder_is_a_load_error() {
    let (_root, package) = package(&[("main.gin", MAIN)]);
    let error = check(&package).unwrap_err();
    assert!(matches!(
        error,
        PackageError::Graph(GraphError::Load(LoadError::ModuleNotFound { .. }))
    ));
}

#[test]
fn an_import_cycle_names_its_path() {
    let (_root, package) = package(&[
        ("main.gin", "use app.a one\n"),
        ("a/a.gin", "use app.b two\n"),
        ("b/b.gin", "use app.a one\n"),
    ]);
    let Err(PackageError::Graph(GraphError::Cycle { path })) = check(&package) else {
        panic!("expected a cycle");
    };
    assert_eq!(path, ["app.a", "app.b", "app.a"]);
}

#[test]
fn a_module_importing_itself_is_a_cycle() {
    let (_root, package) = package(&[("main.gin", "use app one\n")]);
    let Err(PackageError::Graph(GraphError::Cycle { path })) = check(&package) else {
        panic!("expected a cycle");
    };
    assert_eq!(path, ["app", "app"]);
}

#[test]
fn modules_outside_the_package_are_rejected() {
    let (_root, package) = package(&[("main.gin", "use other.util five\n")]);
    assert!(matches!(
        check(&package),
        Err(PackageError::Graph(GraphError::ForeignModule { .. }))
    ));
}

#[test]
fn an_unknown_name_is_attributed_to_its_module_and_file() {
    let (_root, package) = package(&[
        ("main.gin", "use app.util missing\n"),
        ("util/five.gin", UTIL),
    ]);
    let Err(PackageError::Check {
        module,
        path,
        failure,
    }) = check(&package)
    else {
        panic!("expected a check failure");
    };
    assert_eq!(module, "app");
    assert!(path.unwrap().ends_with("main.gin"));
    assert_eq!(failure.error, SourceCheckError::UnknownName);
}

#[test]
fn too_many_modules_fail_before_loading_them_all() {
    let (_root, package) = package(&[
        ("main.gin", "use app.a one\nuse app.b two\n"),
        ("a/a.gin", UTIL),
        ("b/b.gin", UTIL),
    ]);
    let limits = GraphLimits {
        modules_max: 2,
        ..GraphLimits::default()
    };
    assert!(matches!(
        package.load_graph(limits),
        Err(GraphError::TooManyModules { max: 2 })
    ));
}

#[test]
fn zero_module_limit_is_invalid() {
    let (_root, package) = package(&[("main.gin", UTIL)]);
    let limits = GraphLimits {
        modules_max: 0,
        ..GraphLimits::default()
    };
    assert!(matches!(
        package.load_graph(limits),
        Err(GraphError::LimitsInvalid { .. })
    ));
}

#[test]
fn the_root_module_owns_the_entry_and_dependencies_may_not() {
    let (_kept, opened) = package(&[
        (
            "main.gin",
            "use app.util five\n#entry\nstart() in 0 to 16: return five()\n",
        ),
        ("util/five.gin", UTIL),
    ]);
    let checked = check(&opened).unwrap();
    assert_eq!(checked.entry(), Some(0));

    let (_kept, opened) = package(&[
        (
            "main.gin",
            "use app.util five\nagain() in 0 to 16: return five()\n",
        ),
        ("util/five.gin", "#entry\nfive() in 0 to 16: return 5\n"),
    ]);
    assert!(matches!(
        check(&opened),
        Err(PackageError::EntryOutsideRoot { module }) if module == "app.util"
    ));
}
