//! Merge add-import suggestions into existing member/bundle imports from the same root.

use std::path::Path;

use ast::{BundleExportImport, HasSpanId, ImportSource};
use parser::query::SourceParseExt;

use crate::folder_module::normalize_local_import_path;
use crate::import_query::find_package_root;

/// Identifies a mergeable import target (local folder path, flask dependency, or sibling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportRoot {
    LocalPath(String),
    Dependency(String),
    /// `use Foo` / `use Foo, Bar` — symbols from the current folder module.
    CurrentModule,
}

/// A symbol exported from a mergeable root, parsed from existing `use` lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectedImport {
    pub root: ImportRoot,
    pub symbol: String,
    pub alias: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundleMember {
    export: String,
    alias: Option<String>,
}

/// Raw member import before merge (from `find_import_suggestions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSuggestion {
    pub root: ImportRoot,
    pub symbol: String,
}

impl RawSuggestion {
    pub fn format_member_line(&self) -> String {
        match &self.root {
            ImportRoot::Dependency(dep) => format!("use {dep}.{}", self.symbol),
            ImportRoot::LocalPath(path) => format!("use '{path}'.{}", self.symbol),
            ImportRoot::CurrentModule => format!("use {}", self.symbol),
        }
    }
}

/// Result of merging a new symbol into existing imports from the same root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedImportSuggestion {
    pub use_line: String,
    pub replace_line: Option<u32>,
    pub delete_lines: Vec<u32>,
}

/// Parse `source` and collect member-style imports that can be merged.
pub fn collect_member_imports(source: &str) -> Vec<CollectedImport> {
    let output = source.parse_source_full();
    let span_table = &output.ast.span_table;
    let mut out = Vec::new();

    for import in &output.ast.uses {
        for mi in &import.0 {
            match &mi.source {
                ImportSource::LocalMember(m) => {
                    if let Some(path) = &m.local_path {
                        let line = line_index(span_table.get(m.span).start(), source);
                        out.push(CollectedImport {
                            root: ImportRoot::LocalPath(path.to_string_lossy().into_owned()),
                            symbol: m.member.export.to_string(),
                            alias: m.member.alias.map(|a| a.to_string()),
                            line,
                        });
                    }
                }
                ImportSource::LocalBundle(b) => {
                    let line = line_index(span_table.get(b.span_id()).start(), source);
                    let root = if let Some(path) = &b.local_path {
                        ImportRoot::LocalPath(path.to_string_lossy().into_owned())
                    } else {
                        ImportRoot::Dependency(b.root.to_string())
                    };
                    push_bundle_members(&mut out, root, line, &b.members);
                }
                ImportSource::Package(path) if path.segments.len() == 1 => {
                    let line = line_index(span_table.get(path.span_id).start(), source);
                    let seg = &path.segments[0];
                    out.push(CollectedImport {
                        root: ImportRoot::Dependency(path.root.to_string()),
                        symbol: seg.to_string(),
                        alias: None,
                        line,
                    });
                }
                ImportSource::CurrentModule { member } => {
                    let line = line_index(span_table.get(member.span).start(), source);
                    out.push(CollectedImport {
                        root: ImportRoot::CurrentModule,
                        symbol: member.export.to_string(),
                        alias: member.alias.map(|a| a.to_string()),
                        line,
                    });
                }
                _ => {}
            }
        }
    }

    out
}

fn push_bundle_members(
    out: &mut Vec<CollectedImport>,
    root: ImportRoot,
    line: u32,
    members: &[BundleExportImport],
) {
    for member in members {
        out.push(CollectedImport {
            root: root.clone(),
            symbol: member.export.to_string(),
            alias: member.alias.map(|a| a.to_string()),
            line,
        });
    }
}

fn line_index(span_start: usize, source: &str) -> u32 {
    source[..span_start.min(source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count() as u32
}

/// Whether `symbol` is already brought in via a tracked member import.
pub fn symbol_already_imported(symbol: &str, collected: &[CollectedImport]) -> bool {
    collected.iter().any(|c| c.symbol == symbol)
}

impl ImportRoot {
    /// Whether this import root refers to the same module as `other`.
    pub fn matches(&self, other: &ImportRoot, file_path: Option<&Path>) -> bool {
        match (self, other) {
            (ImportRoot::CurrentModule, ImportRoot::CurrentModule) => true,
            (ImportRoot::Dependency(x), ImportRoot::Dependency(y)) => x == y,
            (ImportRoot::LocalPath(x), ImportRoot::LocalPath(y)) => {
                if x == y {
                    return true;
                }
                let Some(file_path) = file_path else {
                    return false;
                };
                let Some(file_dir) = file_path.parent() else {
                    return false;
                };
                let Some(pkg_root) = find_package_root(file_path) else {
                    return false;
                };
                let norm_x = normalize_local_import_path(file_dir, &pkg_root, Path::new(x)).ok();
                let norm_y = normalize_local_import_path(file_dir, &pkg_root, Path::new(y)).ok();
                match (norm_x, norm_y) {
                    (Some(nx), Some(ny)) => nx == ny,
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

/// Format `use A, B` for sibling folder imports (sorted case-insensitively).
fn format_current_module_line(members: &[BundleMember]) -> String {
    let mut sorted: Vec<&BundleMember> = members.iter().collect();
    sorted.sort_by_key(|m| m.export.to_lowercase());

    let member_strs: Vec<String> = sorted
        .iter()
        .map(|m| {
            if let Some(alias) = &m.alias {
                format!("{} as {}", m.export, alias)
            } else {
                m.export.clone()
            }
        })
        .collect();
    format!("use {}", member_strs.join(", "))
}

/// Format a bundle `use` line for the given root and members (sorted case-insensitively).
fn format_bundle_line(root: &ImportRoot, members: &[BundleMember]) -> String {
    let mut sorted: Vec<&BundleMember> = members.iter().collect();
    sorted.sort_by_key(|m| m.export.to_lowercase());

    let member_strs: Vec<String> = sorted
        .iter()
        .map(|m| {
            if let Some(alias) = &m.alias {
                format!("{} as {}", m.export, alias)
            } else {
                m.export.clone()
            }
        })
        .collect();
    let bundle = member_strs.join(", ");

    match root {
        ImportRoot::Dependency(dep) => format!("use {dep}.({bundle})"),
        ImportRoot::LocalPath(path) => format!("use '{path}'.({bundle})"),
        ImportRoot::CurrentModule => format_current_module_line(members),
    }
}

impl RawSuggestion {
    /// Merge this raw suggestion with any existing imports from the same root in `collected`.
    pub fn merge(
        &self,
        collected: &[CollectedImport],
        file_path: Option<&Path>,
    ) -> MergedImportSuggestion {
        let matching: Vec<&CollectedImport> = collected
            .iter()
            .filter(|c| c.root.matches(&self.root, file_path))
            .collect();

        if matching.is_empty() {
            return MergedImportSuggestion {
                use_line: self.format_member_line(),
                replace_line: None,
                delete_lines: Vec::new(),
            };
        }

        let mut members: Vec<BundleMember> = Vec::new();
        let mut lines: Vec<u32> = Vec::new();

        let mut add_member = |export: String, alias: Option<String>| {
            if !members.iter().any(|m| m.export == export) {
                members.push(BundleMember { export, alias });
            }
        };

        add_member(self.symbol.clone(), None);
        for c in &matching {
            lines.push(c.line);
            add_member(c.symbol.clone(), c.alias.clone());
        }

        let display_root = matching
            .iter()
            .map(|c| match &c.root {
                ImportRoot::LocalPath(p) => ImportRoot::LocalPath(p.clone()),
                ImportRoot::Dependency(d) => ImportRoot::Dependency(d.clone()),
                ImportRoot::CurrentModule => ImportRoot::CurrentModule,
            })
            .next()
            .unwrap_or_else(|| self.root.clone());

        let use_line = match &display_root {
            ImportRoot::CurrentModule => format_current_module_line(&members),
            _ => format_bundle_line(&display_root, &members),
        };
        let replace_line = *lines.iter().min().unwrap();
        let mut delete_lines: Vec<u32> = lines.into_iter().filter(|l| *l != replace_line).collect();
        delete_lines.sort_by(|a, b| b.cmp(a));

        MergedImportSuggestion {
            use_line,
            replace_line: Some(replace_line),
            delete_lines,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn format_bundle_line_sorts_case_insensitive() {
        let root = ImportRoot::Dependency("core".to_string());
        let members = vec![
            BundleMember {
                export: "List".to_string(),
                alias: None,
            },
            BundleMember {
                export: "Byte".to_string(),
                alias: None,
            },
        ];
        assert_eq!(format_bundle_line(&root, &members), "use core.(Byte, List)");
    }

    #[test]
    fn collect_parses_local_member_and_bundle() {
        let source = "use '../primitive/'.List\nuse core.(Int, Byte)\n\nmain:\nreturn\n";
        let collected = collect_member_imports(source);
        assert!(collected.iter().any(|c| {
            c.symbol == "List"
                && matches!(&c.root, ImportRoot::LocalPath(p) if p.contains("primitive"))
        }));
        assert!(collected.iter().any(|c| c.symbol == "Int"));
        assert!(collected.iter().any(|c| c.symbol == "Byte"));
    }

    #[test]
    fn merge_combines_local_member_with_new_symbol() {
        let source = "use '../primitive/'.List\n\nString has bytes List(Byte)\n";
        let collected = collect_member_imports(source);
        let raw = RawSuggestion {
            root: ImportRoot::LocalPath("../primitive/".to_string()),
            symbol: "Byte".to_string(),
        };
        let path = Path::new("/tmp/pkg/string/string.gin");
        let merged = raw.merge(&collected, Some(path));
        assert_eq!(merged.use_line, "use '../primitive/'.(Byte, List)");
        assert_eq!(merged.replace_line, Some(0));
        assert!(merged.delete_lines.is_empty());
    }

    #[test]
    fn collect_parses_current_module_imports() {
        let source = "use Marker\nuse Str, Int\n\nmain:\nreturn\n";
        let collected = collect_member_imports(source);
        assert!(
            collected
                .iter()
                .any(|c| { c.symbol == "Marker" && c.root == ImportRoot::CurrentModule })
        );
        assert!(collected.iter().any(|c| c.symbol == "Str"));
        assert!(collected.iter().any(|c| c.symbol == "Int"));
    }

    #[test]
    fn merge_combines_sibling_imports_on_one_line() {
        let source = "use Marker\n\nCopy is Marker(Unit)\n";
        let collected = collect_member_imports(source);
        let raw = RawSuggestion {
            root: ImportRoot::CurrentModule,
            symbol: "Infection".to_string(),
        };
        let merged = raw.merge(&collected, None);
        assert_eq!(merged.use_line, "use Infection, Marker");
        assert_eq!(merged.replace_line, Some(0));
        assert!(merged.delete_lines.is_empty());
    }

    #[test]
    fn merge_combines_dep_member_imports() {
        let source = "use core.Int\n\nmain:\nreturn\n";
        let collected = collect_member_imports(source);
        let raw = RawSuggestion {
            root: ImportRoot::Dependency("core".to_string()),
            symbol: "Byte".to_string(),
        };
        let merged = raw.merge(&collected, None);
        assert_eq!(merged.use_line, "use core.(Byte, Int)");
        assert_eq!(merged.replace_line, Some(0));
    }
}
