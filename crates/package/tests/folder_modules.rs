//! Folder modules: immediate `.gin` files, sorted, parsed with file-naming span ids.
use std::fs;
use std::path::Path;

use package::{LoadError, Package, PackageLimits};

const MANIFEST: &str = "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n";

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
    let package = Package::open(root.path()).expect("manifest opens");
    (root, package)
}

#[test]
fn root_module_takes_the_manifest_name_and_sorted_immediate_files() {
    let (_root, package) = package(&[
        ("b.gin", "second() in 0 to 1: return 1\n"),
        ("a.gin", "first() in 0 to 1: return 0\n"),
        ("notes.txt", "ignored"),
        ("deeper/c.gin", "third() in 0 to 1: return 1\n"),
    ]);
    let module = package.load_module(&[], PackageLimits::default()).unwrap();
    assert_eq!(module.name, "app");
    let names: Vec<_> = module
        .paths
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["a.gin", "b.gin"]);
    for (index, file) in module.files.iter().enumerate() {
        assert_eq!(file.items[0].span_id.file() as usize, index);
    }
}

#[test]
fn nested_folders_are_dotted_module_names() {
    let (_root, package) = package(&[("io/print.gin", "one() in 0 to 1: return 1\n")]);
    let module = package
        .load_module(&["io"], PackageLimits::default())
        .unwrap();
    assert_eq!(module.name, "app.io");
    assert_eq!(module.files.len(), 1);
}

#[test]
fn missing_manifest_and_missing_module_are_distinct_errors() {
    let root = tempfile::tempdir().unwrap();
    assert!(matches!(
        Package::open(root.path()),
        Err(LoadError::ManifestMissing { .. })
    ));
    let (_root, package) = package(&[("a.gin", "x() in 0 to 1: return 1\n")]);
    assert!(matches!(
        package.load_module(&["nope"], PackageLimits::default()),
        Err(LoadError::ModuleNotFound { .. })
    ));
}

#[test]
fn a_folder_without_source_files_is_an_error() {
    let (_root, package) = package(&[("empty/readme.txt", "x")]);
    assert!(matches!(
        package.load_module(&["empty"], PackageLimits::default()),
        Err(LoadError::NoSourceFiles { .. })
    ));
}

#[test]
fn module_segments_cannot_escape_the_package() {
    let (_root, package) = package(&[("a.gin", "x() in 0 to 1: return 1\n")]);
    for segments in [&[".."][..], &[""], &["a/b"], &["a\\b"], &["."]] {
        assert!(
            matches!(
                package.load_module(segments, PackageLimits::default()),
                Err(LoadError::SegmentInvalid { .. })
            ),
            "{segments:?}"
        );
    }
}

#[test]
fn syntax_errors_name_their_file() {
    let (_root, package) = package(&[
        ("a.gin", "fine() in 0 to 1: return 1\n"),
        ("b.gin", "broken Int(\n"),
    ]);
    let Err(LoadError::Syntax { path, diagnostics }) =
        package.load_module(&[], PackageLimits::default())
    else {
        panic!("syntax error expected");
    };
    assert!(path.ends_with("b.gin"));
    assert!(!diagnostics.is_empty());
}

#[test]
fn file_count_and_size_limits_fail_before_parsing() {
    let (_root, package) = package(&[
        ("a.gin", "a() in 0 to 1: return 1\n"),
        ("b.gin", "b() in 0 to 1: return 1\n"),
    ]);
    let few = PackageLimits {
        files_max: 1,
        ..PackageLimits::default()
    };
    assert!(matches!(
        package.load_module(&[], few),
        Err(LoadError::TooManyFiles {
            count: 2,
            max: 1,
            ..
        })
    ));
    let small = PackageLimits {
        file_bytes_max: 4,
        ..PackageLimits::default()
    };
    assert!(matches!(
        package.load_module(&[], small),
        Err(LoadError::FileTooLarge { .. })
    ));
}

#[test]
fn limits_cannot_exceed_what_the_checker_can_address() {
    let (_root, package) = package(&[("a.gin", "a() in 0 to 1: return 1\n")]);
    let huge = PackageLimits {
        files_max: 5_000,
        ..PackageLimits::default()
    };
    assert!(matches!(
        package.load_module(&[], huge),
        Err(LoadError::LimitsInvalid { .. })
    ));
}
