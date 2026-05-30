use std::collections::HashMap;
use std::path::PathBuf;

use parser::{extract_package_import_paths, parse_from_str};
use test_fixtures::TempPackage;

#[test]
fn test_package_import_with_nested_segment() {
    let pkg = TempPackage::new("segment");
    pkg.write_flask("core");
    pkg.write_nested_flask("io", "io_pkg");
    pkg.write("io/print.gin", "print(s Str):\nreturn\n");

    let src = "use core.io\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path().to_path_buf());

    let paths = extract_package_import_paths(&ast, &deps);

    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.ends_with("print.gin"));
}

#[test]
fn test_package_import_no_segments_collects_flat_gin_only() {
    let pkg = TempPackage::new("all");
    pkg.write_flask("core");
    pkg.write("io.gin", "print(s Str):\nreturn\n");
    pkg.write(
        "sys.gin",
        "write(fd Int, buf Int, len Int) Int:\nreturn 0\n",
    );
    pkg.write("readme.md", "not a gin file");
    pkg.write_nested_flask("nested", "nested");
    pkg.write("nested/x.gin", "x\n");

    let src = "use core\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path().to_path_buf());

    let paths = extract_package_import_paths(&ast, &deps);

    let file_names: Vec<_> = paths
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert_eq!(file_names.len(), 2);
    assert!(file_names.contains(&"io.gin".to_string()));
    assert!(file_names.contains(&"sys.gin".to_string()));
    assert!(!file_names.contains(&"readme.md".to_string()));
}

#[test]
fn test_package_import_missing_dependency() {
    let src = "use nonexistent.io\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let deps: HashMap<String, PathBuf> = HashMap::new();
    let paths = extract_package_import_paths(&ast, &deps);

    assert!(paths.is_empty());
}

#[test]
fn test_package_import_multiple_package_uses() {
    let pkg = TempPackage::new("multi");
    pkg.write_flask("core");
    pkg.write("io.gin", "print(s Str):\nreturn\n");
    pkg.write(
        "sys.gin",
        "write(fd Int, buf Int, len Int) Int:\nreturn 0\n",
    );

    let src = "use core\nuse core\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path().to_path_buf());

    let paths = extract_package_import_paths(&ast, &deps);

    let file_names: Vec<_> = paths
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert_eq!(file_names.len(), 4);
}

#[test]
fn test_package_import_skips_local_imports() {
    let pkg = TempPackage::new("mixed");
    pkg.write_flask("core");
    pkg.write("io.gin", "print(s Str):\nreturn\n");

    let src = "use core\nuse './local.gin' as local\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path().to_path_buf());

    let paths = extract_package_import_paths(&ast, &deps);

    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.file_name().unwrap() == "io.gin");
}

#[test]
fn test_package_import_nonexistent_nested_package() {
    let pkg = TempPackage::new("missing_nested");
    pkg.write_flask("core");

    let src = "use core.nonexistent\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path().to_path_buf());

    let paths = extract_package_import_paths(&ast, &deps);

    assert!(paths.is_empty());
}

#[test]
fn test_package_import_bundle_lists_nested_folder_gin_files() {
    let pkg = TempPackage::new("bundle");
    pkg.write_flask("core");
    pkg.write_nested_flask("io", "io_pkg");
    pkg.write_nested_flask("fs", "fs_pkg");
    pkg.write("io/read.gin", "read:\nreturn\n");
    pkg.write("fs/write.gin", "write:\nreturn\n");

    let src = "use core.(io, fs)\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path().to_path_buf());

    let paths = extract_package_import_paths(&ast, &deps);
    let mut names: Vec<_> = paths
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["read.gin".to_string(), "write.gin".to_string()]);
}

#[test]
fn test_package_import_two_segments_nested() {
    let pkg = TempPackage::new("two_seg");
    pkg.write_flask("core");
    pkg.write_nested_flask("io", "io_pkg");
    pkg.write_nested_flask("io/extra", "extra");
    pkg.write("io/extra/z.gin", "z\n");

    let src = "use core.io.extra\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path().to_path_buf());

    let paths = extract_package_import_paths(&ast, &deps);
    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.ends_with("z.gin"));
}
