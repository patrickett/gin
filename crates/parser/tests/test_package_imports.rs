use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use parser::{extract_package_import_paths, parse_from_str};

/// Helper to create a temporary package directory with .gin files.
struct TempPackage {
    dir: PathBuf,
}

impl TempPackage {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("gin_test_pkg_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn add_file(&self, name: &str, contents: &str) {
        fs::write(self.dir.join(name), contents).unwrap();
    }

    fn add_src_file(&self, name: &str, contents: &str) {
        let src_dir = self.dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join(name), contents).unwrap();
    }

    fn path(&self) -> PathBuf {
        self.dir.clone()
    }
}

impl Drop for TempPackage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn test_package_import_with_segment() {
    let pkg = TempPackage::new("segment");
    pkg.add_file(
        "io.gin",
        "print(s Str):\n    write(1, s.pointer as Int, s.len)\nreturn\n",
    );

    let src = "use core.io\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);

    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.file_name().unwrap() == "io.gin");
}

#[test]
fn test_package_import_segment_in_src_dir() {
    let pkg = TempPackage::new("src_segment");
    pkg.add_src_file(
        "int.gin",
        "Int is -9223372036854775808...9223372036854775807\n",
    );

    let src = "use core.int\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);

    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.to_string_lossy().contains("src"));
    assert!(paths[0].0.file_name().unwrap() == "int.gin");
}

#[test]
fn test_package_import_no_segments_collects_all() {
    let pkg = TempPackage::new("all");
    pkg.add_file("io.gin", "print(s Str):\nreturn\n");
    pkg.add_file(
        "sys.gin",
        "write(fd Int, buf Int, len Int) Int:\nreturn 0\n",
    );
    pkg.add_file("readme.md", "not a gin file");
    pkg.add_src_file("bool.gin", "Bool is True or False\n");
    pkg.add_src_file(
        "int.gin",
        "Int is -9223372036854775808...9223372036854775807\n",
    );

    let src = "use core\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);

    let file_names: Vec<_> = paths
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert_eq!(file_names.len(), 4);
    assert!(file_names.contains(&"io.gin".to_string()));
    assert!(file_names.contains(&"sys.gin".to_string()));
    assert!(file_names.contains(&"bool.gin".to_string()));
    assert!(file_names.contains(&"int.gin".to_string()));
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
fn test_package_import_multiple_segments() {
    let pkg = TempPackage::new("multi_seg");
    pkg.add_file("io.gin", "print(s Str):\nreturn\n");
    pkg.add_file(
        "sys.gin",
        "write(fd Int, buf Int, len Int) Int:\nreturn 0\n",
    );

    let src = "use core.io, core.sys\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);

    let file_names: Vec<_> = paths
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert_eq!(file_names.len(), 2);
    assert!(file_names.contains(&"io.gin".to_string()));
    assert!(file_names.contains(&"sys.gin".to_string()));
}

#[test]
fn test_package_import_skips_local_imports() {
    let pkg = TempPackage::new("mixed");
    pkg.add_file("io.gin", "print(s Str):\nreturn\n");

    let src = "use core.io\nuse './local' as local\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);

    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.file_name().unwrap() == "io.gin");
}

#[test]
fn test_package_import_nonexistent_file_in_segment() {
    let pkg = TempPackage::new("missing_file");
    // Don't create any files - the segment resolution should just skip missing files

    let src = "use core.nonexistent\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);

    assert!(paths.is_empty());
}
