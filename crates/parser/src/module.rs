//! Module tree discovery for Gin's import system.
//!
//! A **module** is a directory. Files within the directory are organizational —
//! their bindings are merged into a single flat namespace for that module.
//! Subdirectories become sub-modules accessible via dot qualification.
//!
//! ## Example
//!
//! Given:
//! ```text
//! utils/
//!     error.gin          — defines `Error` tag
//!     requests/
//!         make_request.gin
//!         send_request.gin
//!         internal/
//!             parse_url.gin
//! ```
//!
//! `use 'utils'` makes `Error` available directly and `requests` available
//! as a qualifier. You'd call `requests.make_request(...)` or even
//! `requests.internal.parse_url(...)`.
//!
//! `use 'utils/requests'` makes `make_request` and `send_request` available
//! directly (no qualifier needed).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A node in the module tree representing a single directory.
///
/// Each module has:
/// - **Files**: `.gin` files directly in this directory (merged into one namespace)
/// - **Children**: subdirectories, each becoming a sub-module
#[derive(Debug, Clone)]
pub struct ModuleTree {
    /// The directory name (final path component).
    pub name: String,
    /// `.gin` files directly in this directory, sorted for determinism.
    pub files: Vec<PathBuf>,
    /// Sub-modules keyed by directory name, sorted for determinism.
    pub children: BTreeMap<String, ModuleTree>,
}

// Kinda like the idea of each module having its own flask.json file optionally
// so each module can have its own dependencies and interface hash so you can subscribe to just one module
// or all modules in a project. 

// I think this means implicitly modules cannot inherit from their parent.
// TODO: investigate this

// On second thought, this I think might already work. since child folders are also modules having a flask.json
// would not change how they are used, but you could depend on them from elsewhere easily.

impl ModuleTree {
    /// Returns `true` if this module has no files and no children.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.children.is_empty()
    }

    /// Returns `true` if this module (or any descendant) contains `.gin` files.
    pub fn has_any_files(&self) -> bool {
        !self.files.is_empty() || self.children.values().any(|c| c.has_any_files())
    }

    /// Collect all `.gin` files from this module and every descendant.
    ///
    /// Useful when the compiler needs to parse every file in a module tree.
    pub fn all_files_recursive(&self) -> Vec<PathBuf> {
        let mut result = self.files.clone();
        for child in self.children.values() {
            result.extend(child.all_files_recursive());
        }
        result
    }

    /// Collect only the `.gin` files directly in this module (not sub-modules).
    pub fn direct_files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Resolve a dot-separated qualifier path to a sub-module.
    ///
    /// `resolve_child(&["requests", "internal"])` walks into `requests/` then `internal/`.
    /// Returns `None` if any segment doesn't match a child directory.
    pub fn resolve_child(&self, segments: &[&str]) -> Option<&ModuleTree> {
        let mut current = self;
        for seg in segments {
            current = current.children.get(*seg)?;
        }
        Some(current)
    }

    /// Collect all files reachable through a dot-separated qualifier path.
    ///
    /// If `segments` is empty, returns this module's direct files.
    /// If `segments` is `["requests"]`, returns files from the `requests` sub-module.
    /// Returns `None` if the path doesn't resolve.
    pub fn files_at(&self, segments: &[&str]) -> Option<&[PathBuf]> {
        let module = self.resolve_child(segments)?;
        Some(&module.files)
    }

    /// Return the names of all immediate sub-modules (one level deep).
    pub fn child_names(&self) -> Vec<&str> {
        self.children.keys().map(|s| s.as_str()).collect()
    }
}

/// Directory names to skip during module discovery.
const SKIP_DIRS: &[&str] = &["target", ".git", "node_modules"];

/// Walk a directory and build a `ModuleTree`.
///
/// - `.gin` files in the directory become the module's direct files.
/// - Subdirectories become sub-modules (recursively discovered).
/// - Directories named `target`, `.git`, or `node_modules` are skipped.
/// - Non-`.gin` files are ignored.
///
/// Returns `None` if the directory doesn't exist or can't be read.
pub fn discover_module(dir: &Path) -> Option<ModuleTree> {
    if !dir.is_dir() {
        return None;
    }

    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let entries = std::fs::read_dir(dir).ok()?;

    let mut files = Vec::new();
    let mut children = BTreeMap::new();

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let dir_name = match path.file_name() {
                Some(n) => n.to_string_lossy().into_owned(),
                None => continue,
            };

            // Skip known non-module directories
            if SKIP_DIRS.contains(&dir_name.as_str()) {
                continue;
            }

            // Only discover sub-directories that contain .gin files somewhere
            // in their tree. An empty directory shouldn't show up as a module.
            if let Some(child_tree) = discover_module(&path) {
                if child_tree.has_any_files() {
                    children.insert(dir_name, child_tree);
                }
            }
        } else if path.extension().is_some_and(|e| e == "gin") {
            files.push(path);
        }
    }

    files.sort();

    Some(ModuleTree {
        name,
        files,
        children,
    })
}

/// Discover a module at a specific qualified path within a root directory.
///
/// Given `root = "src"` and `segments = ["utils", "requests"]`, this discovers
/// the module at `src/utils/requests/`. An empty `segments` slice discovers the
/// root directory itself.
///
/// Returns `None` if the path doesn't exist or isn't a directory.
pub fn discover_module_at(root: &Path, segments: &[&str]) -> Option<ModuleTree> {
    let mut dir = root.to_path_buf();
    for seg in segments {
        dir.push(seg);
    }
    discover_module(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("gin_module_test_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn add_file(&self, name: &str, contents: &str) {
            fs::write(self.path.join(name), contents).unwrap();
        }

        fn add_dir(&self, name: &str) -> TempDir {
            let dir = self.path.join(name);
            fs::create_dir_all(&dir).unwrap();
            TempDir { path: dir }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_discover_empty_dir() {
        let tmp = TempDir::new("empty");
        let tree = discover_module(&tmp.path).unwrap();
        assert!(tree.files.is_empty());
        assert!(tree.children.is_empty());
        assert!(tree.is_empty());
    }

    #[test]
    fn test_discover_single_file() {
        let tmp = TempDir::new("single_file");
        tmp.add_file("helper.gin", "foo := 1\n");

        let tree = discover_module(&tmp.path).unwrap();
        assert_eq!(tree.files.len(), 1);
        assert!(tree.files[0].ends_with("helper.gin"));
        assert!(tree.is_empty() == false);
    }

    #[test]
    fn test_discover_ignores_non_gin() {
        let tmp = TempDir::new("non_gin");
        tmp.add_file("readme.md", "hello");
        tmp.add_file("data.json", "{}");

        let tree = discover_module(&tmp.path).unwrap();
        assert!(tree.files.is_empty());
    }

    #[test]
    fn test_discover_submodule() {
        let tmp = TempDir::new("submod");
        tmp.add_file("top.gin", "x := 1\n");
        let sub = tmp.add_dir("requests");
        sub.add_file("make.gin", "make := 1\n");

        let tree = discover_module(&tmp.path).unwrap();
        assert_eq!(tree.files.len(), 1);
        assert!(tree.children.contains_key("requests"));

        let req = &tree.children["requests"];
        assert_eq!(req.files.len(), 1);
        assert!(req.files[0].ends_with("make.gin"));
    }

    #[test]
    fn test_discover_skips_target() {
        let tmp = TempDir::new("skip_target");
        tmp.add_file("main.gin", "main:\nreturn\n");
        let target = tmp.add_dir("target");
        target.add_file("output.o", "binary");

        let tree = discover_module(&tmp.path).unwrap();
        assert_eq!(tree.files.len(), 1);
        assert!(!tree.children.contains_key("target"));
    }

    #[test]
    fn test_discover_nested() {
        let tmp = TempDir::new("nested");
        tmp.add_file("root.gin", "a := 1\n");
        let requests = tmp.add_dir("requests");
        requests.add_file("make.gin", "make := 1\n");
        let internal = requests.add_dir("internal");
        internal.add_file("parse.gin", "parse := 1\n");

        let tree = discover_module(&tmp.path).unwrap();
        assert_eq!(tree.files.len(), 1);

        let req = &tree.children["requests"];
        assert_eq!(req.files.len(), 1);

        let int = &req.children["internal"];
        assert_eq!(int.files.len(), 1);
        assert!(int.files[0].ends_with("parse.gin"));
    }

    #[test]
    fn test_all_files_recursive() {
        let tmp = TempDir::new("recursive");
        tmp.add_file("a.gin", "a\n");
        let sub = tmp.add_dir("sub");
        sub.add_file("b.gin", "b\n");
        let deep = sub.add_dir("deep");
        deep.add_file("c.gin", "c\n");

        let tree = discover_module(&tmp.path).unwrap();
        let all = tree.all_files_recursive();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_resolve_child() {
        let tmp = TempDir::new("resolve");
        let requests = tmp.add_dir("requests");
        let internal = requests.add_dir("internal");
        internal.add_file("parse.gin", "parse\n");

        let tree = discover_module(&tmp.path).unwrap();
        let resolved = tree.resolve_child(&["requests", "internal"]).unwrap();
        assert_eq!(resolved.files.len(), 1);

        assert!(tree.resolve_child(&["nonexistent"]).is_none());
    }

    #[test]
    fn test_empty_subdir_not_discovered() {
        let tmp = TempDir::new("empty_sub");
        tmp.add_file("main.gin", "main\n");
        let _empty = tmp.add_dir("empty_dir");

        let tree = discover_module(&tmp.path).unwrap();
        assert!(!tree.children.contains_key("empty_dir"));
    }

    #[test]
    fn test_discover_module_at() {
        let tmp = TempDir::new("at_path");
        let utils = tmp.add_dir("utils");
        utils.add_file("helper.gin", "h\n");
        let req = utils.add_dir("requests");
        req.add_file("make.gin", "m\n");

        let tree = discover_module_at(&tmp.path, &["utils"]).unwrap();
        assert_eq!(tree.files.len(), 1);
        assert!(tree.children.contains_key("requests"));

        let req_tree = discover_module_at(&tmp.path, &["utils", "requests"]).unwrap();
        assert_eq!(req_tree.files.len(), 1);
        assert!(req_tree.files[0].ends_with("make.gin"));
    }
}
