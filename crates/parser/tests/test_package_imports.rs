use std::collections::HashMap;
use std::path::PathBuf;

use ast::span::SpanId;
use parser::query::{FileAstQueryExt, SourceParseExt};
use test_fixtures::TempPackage;

const CORE: &str = "core";

fn core_flask_package(name: &str) -> TempPackage {
    let package = TempPackage::new(name);
    package.write_flask(CORE);
    package
}

fn core_import_paths(package: &TempPackage, source: &str) -> Vec<(PathBuf, SpanId)> {
    let ast = source.parse_source_full().ast;
    let dependencies = HashMap::from([(CORE.to_string(), package.path().to_path_buf())]);
    ast.extract_package_import_paths(&dependencies)
}

fn file_names(paths: &[(PathBuf, SpanId)]) -> Vec<String> {
    paths
        .iter()
        .map(|(path, _)| path.file_name().unwrap().to_string_lossy().to_string())
        .collect()
}

#[test]
fn test_package_import_with_nested_segment() {
    let package = core_flask_package("segment");
    package.write_nested_flask("io", "io_pkg");
    package.write("io/print.gin", "print(s Str):\n    return\n");

    let paths = core_import_paths(&package, "use core.io\nmain:\n    return\n");

    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.ends_with("print.gin"));
}

#[test]
fn test_package_import_no_segments_collects_flat_gin_only() {
    let package = core_flask_package("all");
    package.write("io.gin", "print(s Str):\n    return\n");
    package.write(
        "sys.gin",
        "write(fd Int, buf Int, len Int) Int:\n    return 0\n",
    );
    package.write("readme.md", "not a gin file");
    package.write_nested_flask("nested", "nested");
    package.write("nested/x.gin", "x\n");

    let paths = core_import_paths(&package, "use core\nmain:\n    return\n");
    let names = file_names(&paths);

    assert_eq!(names.len(), 2);
    assert!(names.contains(&"io.gin".to_string()));
    assert!(names.contains(&"sys.gin".to_string()));
    assert!(!names.contains(&"readme.md".to_string()));
}

#[test]
fn test_package_import_missing_dependency() {
    let src = "use nonexistent.io\nmain:\n    return\n";
    let ast = src.parse_source_full().ast;

    let deps: HashMap<String, PathBuf> = HashMap::new();
    let paths = ast.extract_package_import_paths(&deps);

    assert!(paths.is_empty());
}

#[test]
fn test_package_import_multiple_package_uses() {
    let package = core_flask_package("multi");
    package.write("io.gin", "print(s Str):\n    return\n");
    package.write(
        "sys.gin",
        "write(fd Int, buf Int, len Int) Int:\n    return 0\n",
    );

    let paths = core_import_paths(&package, "use core\nuse core\nmain:\n    return\n");

    assert_eq!(paths.len(), 4);
}

#[test]
fn test_package_import_skips_local_imports() {
    let package = core_flask_package("mixed");
    package.write("io.gin", "print(s Str):\n    return\n");

    let paths = core_import_paths(
        &package,
        "use core\nuse './local.gin' as local\nmain:\n    return\n",
    );

    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.file_name().unwrap() == "io.gin");
}

#[test]
fn test_package_import_nonexistent_nested_package() {
    let package = core_flask_package("missing_nested");

    let paths = core_import_paths(&package, "use core.nonexistent\nmain:\n    return\n");

    assert!(paths.is_empty());
}

#[test]
fn test_package_import_bundle_lists_nested_folder_gin_files() {
    let package = core_flask_package("bundle");
    package.write_nested_flask("io", "io_pkg");
    package.write_nested_flask("fs", "fs_pkg");
    package.write("io/read.gin", "read:\n    return\n");
    package.write("fs/write.gin", "write:\n    return\n");

    let paths = core_import_paths(&package, "use core io, fs\nmain:\n    return\n");
    let mut names = file_names(&paths);
    names.sort();
    assert_eq!(names, vec!["read.gin".to_string(), "write.gin".to_string()]);
}

#[test]
fn test_package_import_two_segments_nested() {
    let package = core_flask_package("two_seg");
    package.write_nested_flask("io", "io_pkg");
    package.write_nested_flask("io/extra", "extra");
    package.write("io/extra/z.gin", "z\n");

    let paths = core_import_paths(&package, "use core.io.extra\nmain:\n    return\n");
    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.ends_with("z.gin"));
}

#[test]
fn minimum_manifests_are_package_boundaries_for_import_queries() {
    let pkg = TempPackage::new("minimum_imports");
    pkg.write("mod.flask", "#settings\nmodule: 'core'\nversion: 0.1.0\n");
    pkg.write("root.gin", "Root is Value\n");
    pkg.write("io/mod.flask", "#settings\nmodule: 'io'\nversion: 0.1.0\n");
    pkg.write("io/print.gin", "Print is Value\n");
    let dependencies = HashMap::from([("core".to_owned(), pkg.path().to_owned())]);
    for (source, file) in [("use core\n", "root.gin"), ("use core.io\n", "print.gin")] {
        let paths = source
            .parse_source_full()
            .ast
            .extract_package_import_paths(&dependencies);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].0.ends_with(file));
    }
    let paths = "use './io/'\n"
        .parse_source_full()
        .ast
        .extract_local_import_paths(pkg.path());
    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.ends_with("print.gin"));
}
