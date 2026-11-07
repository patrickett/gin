//! Path dependencies: a package imports the modules of another package its manifest names.
//!
//! `#dependencies` entries of the form `core: "../core"` name a package by the directory it
//! lives in. An import whose first segment is a dependency's name loads that package's module,
//! below its own root; the dependency is a package like any other, with its own manifest and
//! its own dependencies. Registry and git dependencies are not supported and say so.
use std::fs;
use std::path::Path;

use package::{GraphError, GraphLimits, LoadError, Package, PackageError};
use typecheck::Limits;

fn manifest(name: &str, dependencies: &[(&str, &str)]) -> String {
    let mut text = format!("#settings\nmodule: \"{name}\"\nversion: 0.1.0\n");
    if !dependencies.is_empty() {
        text.push_str("\n#dependencies\n");
        for (key, value) in dependencies {
            text.push_str(&format!("{key}: {value}\n"));
        }
    }
    text
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Sibling packages under one temporary directory, so `"../lib"` means the one next door.
struct Workspace(tempfile::TempDir);

impl Workspace {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn package(&self, name: &str, dependencies: &[(&str, &str)], files: &[(&str, &str)]) {
        let root = self.0.path().join(name);
        write(&root, "mod.flask", &manifest(name, dependencies));
        for (path, text) in files {
            write(&root, path, text);
        }
    }

    fn open(&self, name: &str) -> Package {
        Package::open(&self.0.path().join(name)).expect("manifest opens")
    }
}

fn modules(package: &Package) -> Vec<String> {
    package
        .load_graph(GraphLimits::default())
        .expect("the graph loads")
        .modules
        .iter()
        .map(|module| module.name.clone())
        .collect()
}

const FIVE: &str = "five() in 0 to 16: return 5\n";

#[test]
fn a_package_imports_the_modules_of_its_dependency() {
    let workspace = Workspace::new();
    workspace.package("lib", &[], &[("util/util.gin", FIVE)]);
    workspace.package(
        "app",
        &[("lib", "\"../lib\"")],
        &[(
            "main.gin",
            "use lib.util five\nagain() in 0 to 16: return five()\n",
        )],
    );
    let package = workspace.open("app");
    assert_eq!(modules(&package), ["lib.util", "app"]);
    let checked = package
        .check(GraphLimits::default(), Limits::default())
        .expect("the package checks");
    assert_eq!(checked.capsules.len(), 2);
}

#[test]
fn a_dependency_with_nothing_at_its_root_is_a_library_of_modules() {
    let workspace = Workspace::new();
    workspace.package(
        "lib",
        &[],
        &[
            ("a/a.gin", FIVE),
            (
                "b/b.gin",
                "use lib.a five\nsix() in 0 to 16: return five() + 1\n",
            ),
        ],
    );
    workspace.package(
        "app",
        &[("lib", "\"../lib\"")],
        &[(
            "main.gin",
            "use lib.b six\nseven() in 0 to 16: return six() + 1\n",
        )],
    );
    let package = workspace.open("app");
    assert_eq!(modules(&package), ["lib.a", "lib.b", "app"]);
    package
        .check(GraphLimits::default(), Limits::default())
        .expect("the package checks");
}

#[test]
fn a_dependency_of_a_dependency_is_found_through_the_dependency() {
    let workspace = Workspace::new();
    workspace.package("base", &[], &[("util/util.gin", FIVE)]);
    workspace.package(
        "lib",
        &[("base", "\"../base\"")],
        &[(
            "mid/mid.gin",
            "use base.util five\nsix() in 0 to 16: return five() + 1\n",
        )],
    );
    workspace.package(
        "app",
        &[("lib", "\"../lib\"")],
        &[(
            "main.gin",
            "use lib.mid six\nagain() in 0 to 16: return six()\n",
        )],
    );
    let package = workspace.open("app");
    assert_eq!(modules(&package), ["base.util", "lib.mid", "app"]);
}

/// Each package sees only what its own manifest names: the app does not get `base` through
/// `lib`'s manifest.
#[test]
fn a_dependency_is_not_visible_through_another_packages_manifest() {
    let workspace = Workspace::new();
    workspace.package("base", &[], &[("util/util.gin", FIVE)]);
    workspace.package("lib", &[("base", "\"../base\"")], &[("mid/mid.gin", FIVE)]);
    workspace.package(
        "app",
        &[("lib", "\"../lib\"")],
        &[(
            "main.gin",
            "use base.util five\nagain() in 0 to 16: return five()\n",
        )],
    );
    let error = workspace.open("app").load_graph(GraphLimits::default());
    assert!(
        matches!(error, Err(GraphError::ForeignModule { .. })),
        "{error:?}"
    );
}

#[test]
fn one_dependency_reached_along_two_paths_is_loaded_once() {
    let workspace = Workspace::new();
    workspace.package("base", &[], &[("util/util.gin", FIVE)]);
    for name in ["left", "right"] {
        workspace.package(
            name,
            &[("base", "\"../base\"")],
            &[(
                "m/m.gin",
                "use base.util five\nvalue() in 0 to 16: return five()\n",
            )],
        );
    }
    workspace.package(
        "app",
        &[("left", "\"../left\""), ("right", "\"../right\"")],
        &[(
            "main.gin",
            "use left.m value\nuse right.m\nagain() in 0 to 16: return value()\n",
        )],
    );
    let names = modules(&workspace.open("app"));
    assert_eq!(
        names.iter().filter(|name| *name == "base.util").count(),
        1,
        "{names:?}"
    );
}

#[test]
fn a_dependency_key_must_be_the_name_its_manifest_declares() {
    let workspace = Workspace::new();
    workspace.package("lib", &[], &[("util/util.gin", FIVE)]);
    workspace.package(
        "app",
        &[("other", "\"../lib\"")],
        &[(
            "main.gin",
            "use other.util five\nagain() in 0 to 16: return five()\n",
        )],
    );
    let error = workspace.open("app").load_graph(GraphLimits::default());
    assert!(
        matches!(error, Err(GraphError::DependencyName { .. })),
        "{error:?}"
    );
}

#[test]
fn two_paths_for_one_dependency_name_are_a_conflict() {
    let workspace = Workspace::new();
    workspace.package("lib", &[], &[("util/util.gin", FIVE)]);
    let elsewhere = workspace.0.path().join("elsewhere");
    write(&elsewhere, "lib/mod.flask", &manifest("lib", &[]));
    write(&elsewhere, "lib/util/util.gin", FIVE);
    workspace.package(
        "mid",
        &[("lib", "\"../elsewhere/lib\"")],
        &[(
            "m/m.gin",
            "use lib.util five\nvalue() in 0 to 16: return five()\n",
        )],
    );
    workspace.package(
        "app",
        &[("lib", "\"../lib\""), ("mid", "\"../mid\"")],
        &[(
            "main.gin",
            "use lib.util five\nuse mid.m value\nagain() in 0 to 16: return five()\n",
        )],
    );
    let error = workspace.open("app").load_graph(GraphLimits::default());
    assert!(
        matches!(error, Err(GraphError::DependencyConflict { .. })),
        "{error:?}"
    );
}

#[test]
fn a_registry_dependency_says_it_is_unsupported() {
    let workspace = Workspace::new();
    workspace.package(
        "app",
        &[("lib", "1.2.0")],
        &[(
            "main.gin",
            "use lib.util five\nagain() in 0 to 16: return five()\n",
        )],
    );
    let error = workspace.open("app").load_graph(GraphLimits::default());
    match error {
        Err(GraphError::DependencyUnsupported { name, kind, .. }) => {
            assert_eq!((name.as_str(), kind), ("lib", "registry"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_dependency_whose_directory_is_missing_is_a_load_error() {
    let workspace = Workspace::new();
    workspace.package(
        "app",
        &[("lib", "\"../nowhere\"")],
        &[(
            "main.gin",
            "use lib.util five\nagain() in 0 to 16: return five()\n",
        )],
    );
    let error = workspace.open("app").load_graph(GraphLimits::default());
    assert!(
        matches!(
            error,
            Err(GraphError::Load(LoadError::ManifestMissing { .. }))
        ),
        "{error:?}"
    );
}

#[test]
fn an_import_cycle_across_packages_is_found() {
    let workspace = Workspace::new();
    workspace.package(
        "lib",
        &[("app", "\"../app\"")],
        &[(
            "m/m.gin",
            "use app.back thing\nvalue() in 0 to 16: return thing()\n",
        )],
    );
    workspace.package(
        "app",
        &[("lib", "\"../lib\"")],
        &[
            (
                "main.gin",
                "use lib.m value\nagain() in 0 to 16: return value()\n",
            ),
            (
                "back/back.gin",
                "use lib.m value\nthing() in 0 to 16: return value()\n",
            ),
        ],
    );
    let error = workspace.open("app").load_graph(GraphLimits::default());
    assert!(matches!(error, Err(GraphError::Cycle { .. })), "{error:?}");
}

#[test]
fn only_the_root_package_may_mark_an_entry() {
    let workspace = Workspace::new();
    workspace.package(
        "lib",
        &[],
        &[("util/util.gin", "#entry\nmain() in 0 to 16: return 5\n")],
    );
    workspace.package(
        "app",
        &[("lib", "\"../lib\"")],
        &[("main.gin", "use lib.util\n")],
    );
    let error = workspace
        .open("app")
        .check(GraphLimits::default(), Limits::default());
    assert!(
        matches!(error, Err(PackageError::EntryOutsideRoot { .. })),
        "{error:?}"
    );
}

/// The root of a library with no `.gin` there is not a module; importing it says so.
#[test]
fn importing_a_dependencys_empty_root_is_an_error() {
    let workspace = Workspace::new();
    workspace.package("lib", &[], &[("util/util.gin", FIVE)]);
    workspace.package(
        "app",
        &[("lib", "\"../lib\"")],
        &[("main.gin", "use lib five\nagain() in 0 to 16: return 1\n")],
    );
    let error = workspace.open("app").load_graph(GraphLimits::default());
    assert!(
        matches!(
            error,
            Err(GraphError::Load(LoadError::NoSourceFiles { .. }))
        ),
        "{error:?}"
    );
}

/// A package that names dependencies but imports none of them loads only itself.
#[test]
fn an_unused_dependency_is_not_loaded() {
    let workspace = Workspace::new();
    workspace.package(
        "app",
        &[("lib", "\"../nowhere\"")],
        &[("main.gin", "again() in 0 to 16: return 1\n")],
    );
    assert_eq!(modules(&workspace.open("app")), ["app"]);
}
