//! Suggest `use` statements for symbols defined elsewhere in the project or deps.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use flask::FlaskConfigHandle;

use crate::file_helpers::GinPackageExt;
use crate::folder_module::normalize_local_import_path;
use crate::import_merge::{
    ImportRoot, RawSuggestion, collect_member_imports, symbol_already_imported,
};
use crate::import_query::find_package_root;

/// A single import fix: a full `use …` line (no trailing newline).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSuggestion {
    pub use_line: String,
    pub replace_line: Option<u32>,
    pub delete_lines: Vec<u32>,
}

/// Find import paths that would bring `symbol` into scope at `file_path`.
///
/// Search order: same folder module, same package (local path), then declared
/// path dependencies. Results are deduplicated and omit symbols already imported
/// in `source`.
pub fn find_import_suggestions(
    file_path: &Path,
    symbol: &str,
    source: &str,
) -> Vec<ImportSuggestion> {
    let file_dir = match file_path.parent() {
        Some(d) => d,
        None => return Vec::new(),
    };

    let collected = collect_member_imports(source);
    if symbol_already_imported(symbol, &collected) {
        return Vec::new();
    }

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    // Same folder module (sibling `.gin` files).
    if crate::public_symbols::find_public_def(file_dir, symbol, false).is_some() {
        push_merged_import(
            &mut seen,
            &mut out,
            &collected,
            file_path,
            ImportRoot::CurrentModule,
            symbol,
        );
    }

    let dependencies = dependency_dirs(file_dir);

    // Same package, different folder module — but not symbols that live only in a
    // nested path-dependency package (those are imported as `use dep.Symbol`).
    if let Some(pkg_root) = find_package_root(file_path)
        && let Some(def_file) = pkg_root.find_public_def(symbol)
    {
        let def_dir = match def_file.parent() {
            Some(d) => d,
            None => return out,
        };
        let in_dependency_subpackage = dependencies
            .values()
            .any(|dep_dir| def_file.starts_with(dep_dir));
        if !in_dependency_subpackage
            && def_dir != file_dir
            && let Some(local_path) = local_import_path_for_module(file_dir, &pkg_root, def_dir)
            && normalize_local_import_path(file_dir, &pkg_root, Path::new(&local_path)).is_ok()
        {
            push_merged_import(
                &mut seen,
                &mut out,
                &collected,
                file_path,
                ImportRoot::LocalPath(local_path),
                symbol,
            );
        }
    }

    // Path dependencies from flask.jsonc.
    for (dep_name, dep_dir) in dependencies {
        if dep_dir.find_public_def(symbol).is_some() {
            push_merged_import(
                &mut seen,
                &mut out,
                &collected,
                file_path,
                ImportRoot::Dependency(dep_name),
                symbol,
            );
        }
    }

    out
}

fn push_merged_import(
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<ImportSuggestion>,
    collected: &[crate::import_merge::CollectedImport],
    file_path: &Path,
    root: ImportRoot,
    symbol: &str,
) {
    let raw = RawSuggestion {
        root,
        symbol: symbol.to_string(),
    };
    let merged = raw.merge(collected, Some(file_path));
    if seen.insert(merged.use_line.clone()) {
        out.push(ImportSuggestion {
            use_line: merged.use_line,
            replace_line: merged.replace_line,
            delete_lines: merged.delete_lines,
        });
    }
}

fn dependency_dirs(file_dir: &Path) -> HashMap<String, PathBuf> {
    let handle = match FlaskConfigHandle::load(file_dir) {
        Ok(h) => h,
        Err(_) => return HashMap::new(),
    };
    let cfg = handle.read();
    let config_dir = handle.source_dir();
    config_dir.resolve_flask_dependencies(&cfg.config)
}

/// Quoted import path (no surrounding quotes) from `from_dir` to folder module `target_dir`.
fn local_import_path_for_module(
    from_dir: &Path,
    pkg_root: &Path,
    target_dir: &Path,
) -> Option<String> {
    // Prefer a path relative to the importing file's directory.
    if let Some(rel) = path_relative(from_dir, target_dir)
        && normalize_local_import_path(from_dir, pkg_root, Path::new(&rel)).is_ok()
    {
        return Some(rel);
    }

    // Fall back to path from package root (e.g. `primitive/` from `io/`).
    if let Ok(rel) = target_dir.strip_prefix(pkg_root) {
        let components: Vec<_> = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();
        if !components.is_empty() {
            let mut from_root = components.join("/");
            if !from_root.ends_with('/') {
                from_root.push('/');
            }
            if normalize_local_import_path(from_dir, pkg_root, Path::new(&from_root)).is_ok() {
                return Some(from_root);
            }
        }
    }

    None
}

fn path_relative(from: &Path, to: &Path) -> Option<String> {
    use std::path::Component;

    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let mut i = 0usize;
    while i < from.len() && i < to.len() && from[i] == to[i] {
        i += 1;
    }
    if i == from.len() && i == to.len() {
        return Some("./".to_string());
    }
    let mut parts: Vec<String> = (0..from.len().saturating_sub(i))
        .map(|_| "..".to_string())
        .collect();
    for c in &to[i..] {
        if let Component::Normal(s) = c {
            parts.push(s.to_string_lossy().into_owned());
        }
    }
    let mut s = parts.join("/");
    if !s.ends_with('/') {
        s.push('/');
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use test_fixtures::TempPackage;

    #[test]
    fn suggests_dependency_member_import() {
        let pkg = TempPackage::new("dep");
        pkg.write(
            "flask.jsonc",
            r#"{"name":"app","version":"0.0.0","authors":[],"dependencies":{"core":{"path":"core"}}}"#,
        );
        pkg.write(
            "core/flask.jsonc",
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        pkg.write("core/int.gin", "Int is Unit\n");
        pkg.write("main.gin", "main:\nreturn\n");

        let source = "main:\nreturn\n";
        let suggestions = find_import_suggestions(&pkg.join("main.gin"), "Int", source);
        assert!(
            suggestions.iter().any(|s| s.use_line == "use core.Int"),
            "expected `use core.Int`, got {:?}",
            suggestions
        );
    }

    #[test]
    fn suggests_sibling_use_for_same_folder() {
        let pkg = TempPackage::new("sibling");
        pkg.write(
            "flask.jsonc",
            r#"{"name":"app","version":"0.0.0","authors":[]}"#,
        );
        pkg.write("a/one.gin", "Foo is Unit\n");
        pkg.write("a/two.gin", "main:\nreturn\n");

        let source = "main:\nreturn\n";
        let suggestions = find_import_suggestions(&pkg.path().join("a/two.gin"), "Foo", source);
        assert_eq!(
            suggestions,
            vec![ImportSuggestion {
                use_line: "use Foo".to_string(),
                replace_line: None,
                delete_lines: vec![],
            }]
        );
    }

    #[test]
    fn skips_suggestions_already_in_source() {
        let pkg = TempPackage::new("existing");
        pkg.write(
            "flask.jsonc",
            r#"{"name":"app","version":"0.0.0","authors":[],"dependencies":{"core":{"path":"core"}}}"#,
        );
        pkg.write(
            "core/flask.jsonc",
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        pkg.write("core/int.gin", "Int is Unit\n");
        let source = "use core.Int\n\nmain:\nreturn\n";

        let suggestions = find_import_suggestions(&pkg.path().join("main.gin"), "Int", source);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn suggests_to_string_for_bool_provided_trait_across_modules() {
        let pkg = TempPackage::new("core");
        pkg.write(
            "flask.jsonc",
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        pkg.write("string/string.gin", "ToString has to_string String\n");
        pkg.write(
            "primitive/bool.gin",
            "Bool is True or False and\n    has ToString(to_string: when self then 'true' else 'false')\n",
        );

        let source = fs::read_to_string(pkg.path().join("primitive/bool.gin")).unwrap();
        let suggestions =
            find_import_suggestions(&pkg.path().join("primitive/bool.gin"), "ToString", &source);
        assert!(
            suggestions.iter().any(|s| {
                s.use_line.contains("ToString")
                    && (s.use_line.contains("string") || s.use_line.contains("String"))
            }),
            "bool.gin should get a ToString import from string/, got {suggestions:?}"
        );
    }

    #[test]
    fn suggests_local_path_across_folders_like_gin_core() {
        let pkg = TempPackage::new("xpkg");
        pkg.write(
            "flask.jsonc",
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        pkg.write("string/string.gin", "ToString has to_string String\n");
        pkg.write("primitive/bool.gin", "Bool is True or False\n");

        let source = fs::read_to_string(pkg.path().join("primitive/bool.gin")).unwrap();
        let suggestions =
            find_import_suggestions(&pkg.path().join("primitive/bool.gin"), "ToString", &source);
        assert!(
            suggestions.iter().any(|s| s.use_line.contains("ToString")),
            "expected a ToString import, got {:?}",
            suggestions
        );
    }

    #[test]
    fn merges_sibling_import_when_adding_second_symbol() {
        let pkg = TempPackage::new("merge_sibling");
        pkg.write(
            "flask.jsonc",
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        pkg.write(
            "marker/marker.gin",
            "Marker has infect Unit\nInfection is Unit\n",
        );
        let source = "use Marker\n\nCopy is Marker(Unit)\n";
        pkg.write("marker/copy.gin", source);

        let path = pkg.path().join("marker/copy.gin");
        let suggestions = find_import_suggestions(&path, "Infection", source);
        let merged = suggestions
            .iter()
            .find(|s| s.use_line.contains("Infection") && s.use_line.contains("Marker"))
            .expect("expected merged sibling import");
        assert_eq!(merged.use_line, "use Infection, Marker");
        assert_eq!(merged.replace_line, Some(0));
        assert!(merged.delete_lines.is_empty());
    }

    #[test]
    fn merges_local_member_import_when_adding_second_symbol() {
        let pkg = TempPackage::new("merge_local");
        pkg.write(
            "flask.jsonc",
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        pkg.write("primitive/list.gin", "List(x) is Unit\n");
        pkg.write("primitive/int.gin", "Byte is Unit\n");
        let source = "use '../primitive/'.List\n\nString has bytes List(Byte)\n";
        pkg.write("string/string.gin", source);

        let path = pkg.path().join("string/string.gin");
        let suggestions = find_import_suggestions(&path, "Byte", source);
        let merged = suggestions
            .iter()
            .find(|s| s.use_line.contains("Byte") && s.use_line.contains("List"))
            .expect("expected merged primitive import");
        assert_eq!(merged.use_line, "use '../primitive/'.(Byte, List)");
        assert_eq!(merged.replace_line, Some(0));
        assert!(merged.delete_lines.is_empty());
    }

    #[test]
    fn merges_dep_member_import_when_adding_second_symbol() {
        let pkg = TempPackage::new("merge_dep");
        pkg.write(
            "flask.jsonc",
            r#"{"name":"app","version":"0.0.0","authors":[],"dependencies":{"core":{"path":"core"}}}"#,
        );
        pkg.write(
            "core/flask.jsonc",
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        pkg.write("core/int.gin", "Int is Unit\n");
        pkg.write("core/byte.gin", "Byte is Unit\n");
        let source = "use core.Int\n\nmain:\nreturn\n";
        pkg.write("main.gin", source);

        let suggestions = find_import_suggestions(&pkg.path().join("main.gin"), "Byte", source);
        let merged = suggestions
            .iter()
            .find(|s| s.use_line.contains("Byte") && s.use_line.contains("Int"))
            .expect("expected merged core import");
        assert_eq!(merged.use_line, "use core.(Byte, Int)");
        assert_eq!(merged.replace_line, Some(0));
    }
}
