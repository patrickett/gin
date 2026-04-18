use std::fs;
use std::path::PathBuf;

use parser::{discover_module, discover_module_at, parse_from_str};

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("gin_mod_import_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self) -> &std::path::Path {
        &self.root
    }

    fn add_file(&self, relative_path: &str, contents: &str) {
        let full = self.root.join(relative_path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, contents).unwrap();
    }

    fn add_dir(&self, relative_path: &str) {
        fs::create_dir_all(self.root.join(relative_path)).unwrap();
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_module_tree_flat_directory() {
    let proj = TempProject::new("flat");
    proj.add_file("utils/helper.gin", "helper := 1\n");
    proj.add_file("utils/format.gin", "format := 2\n");

    let tree = discover_module(&proj.path().join("utils")).unwrap();

    assert_eq!(tree.files.len(), 2);
    assert!(tree.children.is_empty());
    let names: Vec<_> = tree
        .files
        .iter()
        .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    assert!(names.contains(&"format.gin".to_string()));
    assert!(names.contains(&"helper.gin".to_string()));
}

#[test]
fn test_module_tree_with_submodule() {
    let proj = TempProject::new("submod");
    proj.add_file("utils/error.gin", "Error is None\n");
    proj.add_file("utils/requests/make_request.gin", "make := 1\n");
    proj.add_file("utils/requests/send_request.gin", "send := 2\n");

    let tree = discover_module(&proj.path().join("utils")).unwrap();

    // Direct files: error.gin only
    assert_eq!(tree.files.len(), 1);
    assert!(tree.files[0].ends_with("error.gin"));

    // Sub-module: requests
    assert_eq!(tree.children.len(), 1);
    let requests = &tree.children["requests"];
    assert_eq!(requests.files.len(), 2);
}

#[test]
fn test_module_tree_nested_submodules() {
    let proj = TempProject::new("nested");
    proj.add_file("utils/error.gin", "Error is None\n");
    proj.add_file("utils/requests/make.gin", "make := 1\n");
    proj.add_file("utils/requests/internal/parse_url.gin", "parse := 1\n");

    let tree = discover_module(&proj.path().join("utils")).unwrap();

    // Top level: just error.gin
    assert_eq!(tree.files.len(), 1);
    assert!(tree.children.contains_key("requests"));

    let requests = &tree.children["requests"];
    assert_eq!(requests.files.len(), 1);
    assert!(requests.children.contains_key("internal"));

    let internal = &requests.children["internal"];
    assert_eq!(internal.files.len(), 1);
    assert!(internal.files[0].ends_with("parse_url.gin"));
}

#[test]
fn test_all_files_recursive_collects_everything() {
    let proj = TempProject::new("all_files");
    proj.add_file("utils/a.gin", "a\n");
    proj.add_file("utils/requests/b.gin", "b\n");
    proj.add_file("utils/requests/internal/c.gin", "c\n");
    proj.add_file("utils/helpers/d.gin", "d\n");

    let tree = discover_module(&proj.path().join("utils")).unwrap();
    let all = tree.all_files_recursive();

    assert_eq!(all.len(), 4);
}

#[test]
fn test_direct_files_excludes_submodules() {
    let proj = TempProject::new("direct_only");
    proj.add_file("utils/top.gin", "top\n");
    proj.add_file("utils/sub/child.gin", "child\n");

    let tree = discover_module(&proj.path().join("utils")).unwrap();

    assert_eq!(tree.direct_files().len(), 1);
    assert!(tree.direct_files()[0].ends_with("top.gin"));
}

#[test]
fn test_resolve_child_walks_path() {
    let proj = TempProject::new("resolve");
    proj.add_file("root.gin", "r\n");
    proj.add_file("a/b/c/deep.gin", "deep\n");

    let tree = discover_module(proj.path()).unwrap();

    // Root has child "a"
    let a = tree.resolve_child(&["a"]).unwrap();
    assert!(a.children.contains_key("b"));

    // a has child "b"
    let b = a.resolve_child(&["b"]).unwrap();
    assert!(b.children.contains_key("c"));

    // Walk the full path at once
    let deep = tree.resolve_child(&["a", "b", "c"]).unwrap();
    assert_eq!(deep.files.len(), 1);
    assert!(deep.files[0].ends_with("deep.gin"));

    // Nonexistent path
    assert!(tree.resolve_child(&["nonexistent"]).is_none());
}

#[test]
fn test_child_names() {
    let proj = TempProject::new("child_names");
    proj.add_file("alpha/a.gin", "a\n");
    proj.add_file("beta/b.gin", "b\n");

    let tree = discover_module(proj.path()).unwrap();
    let names = tree.child_names();

    assert_eq!(names, vec!["alpha", "beta"]);
}

#[test]
fn test_empty_directories_not_discovered_as_children() {
    let proj = TempProject::new("empty_dir");
    proj.add_file("main.gin", "main\n");
    proj.add_dir("empty_subdir");

    let tree = discover_module(proj.path()).unwrap();

    assert_eq!(tree.files.len(), 1);
    assert!(!tree.children.contains_key("empty_subdir"));
}

#[test]
fn test_target_directory_skipped() {
    let proj = TempProject::new("skip_target");
    proj.add_file("src/main.gin", "main\n");
    proj.add_file("target/output.o", "binary");

    let tree = discover_module(proj.path()).unwrap();
    assert!(!tree.children.contains_key("target"));
}

#[test]
fn test_discover_module_at_qualified_path() {
    let proj = TempProject::new("at_path");
    proj.add_file("src/utils/io.gin", "io\n");
    proj.add_file("src/utils/requests/make.gin", "make\n");

    let src_tree = discover_module_at(proj.path(), &["src"]).unwrap();
    assert!(src_tree.children.contains_key("utils"));

    let utils = discover_module_at(proj.path(), &["src", "utils"]).unwrap();
    assert_eq!(utils.files.len(), 1);
    assert!(utils.children.contains_key("requests"));

    let requests = discover_module_at(proj.path(), &["src", "utils", "requests"]).unwrap();
    assert_eq!(requests.files.len(), 1);
}

#[test]
fn test_nonexistent_directory_returns_none() {
    let result = discover_module(&std::path::Path::new("/nonexistent/path/xyz"));
    assert!(result.is_none());
}

#[test]
fn test_use_local_import_parses_correctly() {
    let src = "use 'utils'\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    assert_eq!(ast.uses().len(), 1);
    let import = &ast.uses()[0];
    assert_eq!(import.0.len(), 1);

    match &import.0[0].source {
        ast::ImportSource::Local(path, _) => {
            assert_eq!(path.to_string_lossy(), "utils");
        }
        _ => panic!("expected local import"),
    }
}

#[test]
fn test_use_local_subpath_parses_correctly() {
    let src = "use 'utils/requests'\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    assert_eq!(ast.uses().len(), 1);
    let import = &ast.uses()[0];

    match &import.0[0].source {
        ast::ImportSource::Local(path, _) => {
            assert_eq!(path.to_string_lossy(), "utils/requests");
        }
        _ => panic!("expected local import"),
    }
}

#[test]
fn test_use_local_with_alias() {
    let src = "use 'utils' as u\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    let import = &ast.uses()[0];
    assert!(import.0[0].alias.is_some());
}

#[test]
fn test_use_multiple_local_imports() {
    let src = "use 'utils', 'helpers'\nmain:\nreturn\n";
    let ast = parse_from_str(src);

    assert_eq!(ast.uses().len(), 1);
    assert_eq!(ast.uses()[0].0.len(), 2);
}

#[test]
fn test_module_tree_files_sorted_deterministically() {
    let proj = TempProject::new("sorted");
    proj.add_file("mod/zebra.gin", "z\n");
    proj.add_file("mod/alpha.gin", "a\n");
    proj.add_file("mod/middle.gin", "m\n");

    let tree = discover_module(&proj.path().join("mod")).unwrap();

    assert_eq!(tree.files.len(), 3);
    let names: Vec<_> = tree
        .files
        .iter()
        .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();

    // Files should be sorted
    assert_eq!(names[0], "alpha.gin");
    assert_eq!(names[1], "middle.gin");
    assert_eq!(names[2], "zebra.gin");
}

#[test]
fn test_module_tree_children_sorted_deterministically() {
    let proj = TempProject::new("sorted_children");
    proj.add_file("z_sub/a.gin", "a\n");
    proj.add_file("a_sub/b.gin", "b\n");
    proj.add_file("m_sub/c.gin", "c\n");

    let tree = discover_module(proj.path()).unwrap();
    let names: Vec<_> = tree.children.keys().collect();

    assert_eq!(names[0].as_str(), "a_sub");
    assert_eq!(names[1].as_str(), "m_sub");
    assert_eq!(names[2].as_str(), "z_sub");
}

#[test]
fn test_files_at_returns_direct_files_for_submodule() {
    let proj = TempProject::new("files_at");
    proj.add_file("root.gin", "r\n");
    proj.add_file("sub/a.gin", "a\n");
    proj.add_file("sub/b.gin", "b\n");

    let tree = discover_module(proj.path()).unwrap();

    let sub_files = tree.files_at(&["sub"]).unwrap();
    assert_eq!(sub_files.len(), 2);
}

#[test]
fn test_has_any_files() {
    let proj = TempProject::new("has_files");
    proj.add_dir("empty");
    proj.add_file("has_some/x.gin", "x\n");

    let tree = discover_module(proj.path()).unwrap();

    // The empty dir should not appear as a child
    assert!(!tree.children.contains_key("empty"));
    assert!(tree.children.contains_key("has_some"));
}
