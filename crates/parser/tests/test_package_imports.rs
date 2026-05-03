use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use parser::{extract_package_import_paths, parse_from_str};

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

    fn add_nested_flask(&self, relative_dir: &str, name: &str) {
        let d = self.dir.join(relative_dir);
        fs::create_dir_all(&d).unwrap();
        let json = format!(r#"{{"name":"{name}","version":"0.1.0","authors":[]}}"#);
        fs::write(d.join("flask.jsonc"), json).unwrap();
    }

    fn path(&self) -> PathBuf {
        self.dir.clone()
    }

    fn add_flask_root(&self, name: &str) {
        let json = format!(r#"{{"name":"{name}","version":"0.1.0","authors":[]}}"#);
        fs::write(self.dir.join("flask.jsonc"), json).unwrap();
    }
}

impl Drop for TempPackage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn test_package_import_with_nested_segment() {
    let pkg = TempPackage::new("segment");
    pkg.add_flask_root("core");
    pkg.add_nested_flask("io", "io_pkg");
    pkg.add_file("io/print.gin", "print(s Str):\nreturn\n");

    let src = "use core.io\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);

    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.ends_with("print.gin"));
}

#[test]
fn test_package_import_no_segments_collects_flat_gin_only() {
    let pkg = TempPackage::new("all");
    pkg.add_flask_root("core");
    pkg.add_file("io.gin", "print(s Str):\nreturn\n");
    pkg.add_file(
        "sys.gin",
        "write(fd Int, buf Int, len Int) Int:\nreturn 0\n",
    );
    pkg.add_file("readme.md", "not a gin file");
    pkg.add_nested_flask("nested", "nested");
    pkg.add_file("nested/x.gin", "x\n");

    let src = "use core\nmain:\nreturn\n";
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
    pkg.add_flask_root("core");
    pkg.add_file("io.gin", "print(s Str):\nreturn\n");
    pkg.add_file(
        "sys.gin",
        "write(fd Int, buf Int, len Int) Int:\nreturn 0\n",
    );

    let src = "use core\nuse core\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

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
    pkg.add_flask_root("core");
    pkg.add_file("io.gin", "print(s Str):\nreturn\n");

    let src = "use core\nuse './local.gin' as local\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);

    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.file_name().unwrap() == "io.gin");
}

#[test]
fn test_package_import_nonexistent_nested_package() {
    let pkg = TempPackage::new("missing_nested");
    pkg.add_flask_root("core");

    let src = "use core.nonexistent\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);

    assert!(paths.is_empty());
}

#[test]
fn test_package_import_bundle_lists_nested_folder_gin_files() {
    let pkg = TempPackage::new("bundle");
    pkg.add_flask_root("core");
    pkg.add_nested_flask("io", "io_pkg");
    pkg.add_nested_flask("fs", "fs_pkg");
    pkg.add_file("io/read.gin", "read:\nreturn\n");
    pkg.add_file("fs/write.gin", "write:\nreturn\n");

    let src = "use core.(io, fs)\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

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
    pkg.add_flask_root("core");
    pkg.add_nested_flask("io", "io_pkg");
    pkg.add_nested_flask("io/extra", "extra");
    pkg.add_file("io/extra/z.gin", "z\n");

    let src = "use core.io.extra\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.path());

    let paths = extract_package_import_paths(&ast, &deps);
    assert_eq!(paths.len(), 1);
    assert!(paths[0].0.ends_with("z.gin"));
}
