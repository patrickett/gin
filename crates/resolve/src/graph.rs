//! Import graph construction — build the file dependency graph for a package.
//!
//! This phase collects all `.gin` files (entry + dependencies), parses any
//! unparsed dependency files, discovers their imports, and produces a
//! [`ResolveGraph`] describing the file-level dependency structure.
//!
//! Import resolution (matching import statements to files) is delegated to
//! [`super::resolve_module_import`] and related functions in `package_resolver.rs`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ast::{HasSpanId, ImportSource, SymbolAlias};
use diagnostic::{Diagnostic, Span};
use flask::PACKAGE_CONFIG_NAME;

use parser::query::SourceParseExt;

use crate::ParsedFile;
use crate::file_helpers::GinPackageExt;
use crate::module_graph::{ImportEdge, detect_first_cycle};
use crate::package_resolver::resolve_module_import;

/// A single file in the import graph.
#[derive(Debug, Clone)]
pub struct ResolveNode {
    pub path: PathBuf,
    pub qualifier: String,
}

/// The full import dependency graph for a package.
#[derive(Debug, Clone)]
pub struct ResolveGraph {
    pub nodes: Vec<ResolveNode>,
    pub adj: Vec<Vec<ImportEdge>>,
    pub node_aliases: Vec<Vec<SymbolAlias>>,
    pub symptoms: Vec<(usize, Diagnostic)>,
}

struct PublicSymbolIndex {
    by_module: HashMap<PathBuf, HashMap<String, PathBuf>>,
    canonical_modules: HashMap<PathBuf, Option<PathBuf>>,
}

impl PublicSymbolIndex {
    fn from_parsed_files(files: &HashMap<PathBuf, ParsedFile>) -> Self {
        let mut paths: Vec<&PathBuf> = files.keys().collect();
        paths.sort();

        let mut index = Self {
            by_module: HashMap::new(),
            canonical_modules: HashMap::new(),
        };
        for path in paths {
            let ast = &files[path].output.ast;
            for name in ast.defs.keys() {
                if !ast.private_defs.contains(name) {
                    index.insert(path, name.as_str());
                }
            }
            for name in ast.tags.keys() {
                if !ast.private_tags.contains(name) {
                    index.insert(path, name.as_str());
                }
            }
        }
        index
    }

    fn insert(&mut self, file_path: &Path, symbol: &str) {
        let Some(parent) = file_path.parent() else {
            return;
        };

        self.insert_in_module(parent, symbol, file_path);

        let mut ancestor = parent.parent();
        while let Some(module_dir) = ancestor {
            if module_dir.join(PACKAGE_CONFIG_NAME).is_file() {
                self.insert_in_module(module_dir, symbol, file_path);
            }
            ancestor = module_dir.parent();
        }
    }

    fn insert_in_module(&mut self, module_dir: &Path, symbol: &str, file_path: &Path) {
        for module_key in self.module_keys(module_dir) {
            self.by_module
                .entry(module_key)
                .or_default()
                .entry(symbol.to_string())
                .or_insert_with(|| file_path.to_path_buf());
        }
    }

    fn module_keys(&mut self, module_dir: &Path) -> Vec<PathBuf> {
        let module_dir = module_dir.to_path_buf();
        let canonical = self
            .canonical_modules
            .entry(module_dir.clone())
            .or_insert_with(|| module_dir.canonicalize().ok())
            .clone();

        let mut keys = vec![module_dir.clone()];
        if let Some(canonical) = canonical
            && canonical != module_dir
        {
            keys.push(canonical);
        }
        keys
    }

    fn find(&self, module_dir: &Path, symbol: &str) -> Option<PathBuf> {
        self.by_module
            .get(module_dir)
            .or_else(|| {
                module_dir
                    .canonicalize()
                    .ok()
                    .and_then(|path| self.by_module.get(&path))
            })
            .and_then(|symbols| symbols.get(symbol))
            .cloned()
    }
}

/// Collect all `.gin` files for a package, parse any that aren't already
/// parsed, discover their imports, and build the [`ResolveGraph`].
pub(crate) fn build_import_closure(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
) -> (ResolveGraph, HashMap<PathBuf, ParsedFile>) {
    let entry_paths: Vec<PathBuf> = entry_files.iter().map(|f| f.path.clone()).collect();

    let dependency_roots: HashSet<String> = entry_files
        .iter()
        .flat_map(|file| file.output.ast.uses.iter())
        .flat_map(|use_stmt| use_stmt.0.iter())
        .filter_map(|module_import| match &module_import.source {
            ImportSource::Package(path) => Some(path.root.to_string()),
            ImportSource::LocalBundle(bundle) if bundle.local_path.is_none() => {
                Some(bundle.root.to_string())
            }
            ImportSource::Local(..)
            | ImportSource::LocalBundle(..)
            | ImportSource::CurrentModule { .. }
            | ImportSource::LocalMember(..) => None,
        })
        .collect();

    let mut all_paths = entry_paths.clone();
    for dependency_root in dependency_roots {
        if let Some(dep_dir) = dependencies.get(&dependency_root) {
            all_paths.extend(dep_dir.collect_gin_files());
        }
    }
    all_paths.sort();
    all_paths.dedup();

    let mut available: HashMap<PathBuf, ParsedFile> = entry_files
        .into_iter()
        .map(|f| (f.path.clone(), f))
        .collect();
    for path in &all_paths {
        if !available.contains_key(path)
            && let Ok(source) = std::fs::read_to_string(path)
        {
            let output = source.parse_source_full();
            available.insert(
                path.clone(),
                ParsedFile {
                    path: path.clone(),
                    source,
                    output,
                },
            );
        }
    }

    let public_symbols = PublicSymbolIndex::from_parsed_files(&available);
    let graph = discovery(&available, &entry_paths, dependencies, &|dir, sym| {
        public_symbols.find(dir, sym)
    });

    (graph, available)
}

/// Discover imports and build the full dependency graph.
///
/// Processes nodes iteratively, resolving each file's imports and adding
/// newly discovered files to the graph. Detects cycles and duplicate qualifiers.
pub(crate) fn discovery(
    available: &HashMap<PathBuf, ParsedFile>,
    entry_paths: &[PathBuf],
    deps: &HashMap<String, PathBuf>,
    find_public_def: &dyn Fn(&Path, &str) -> Option<PathBuf>,
) -> ResolveGraph {
    let mut nodes: Vec<ResolveNode> = Vec::new();
    let mut adj: Vec<Vec<ImportEdge>> = Vec::new();
    let mut node_aliases: Vec<Vec<SymbolAlias>> = Vec::new();
    let mut symptoms: Vec<(usize, Diagnostic)> = Vec::new();
    let mut seen: HashMap<PathBuf, String> = HashMap::new();
    let mut node_by_path: HashMap<PathBuf, usize> = HashMap::new();
    let mut processed: Vec<bool> = Vec::new();

    for entry_path in entry_paths {
        let path = entry_path.clone();
        if !seen.contains_key(&path) {
            seen.insert(path.clone(), String::new());
            node_by_path.insert(path.clone(), nodes.len());
            nodes.push(ResolveNode {
                path,
                qualifier: String::new(),
            });
            adj.push(Vec::new());
            node_aliases.push(Vec::new());
            processed.push(false);
        }
    }

    loop {
        let next = processed
            .iter()
            .enumerate()
            .find_map(|(i, done)| (!done).then_some(i));
        let Some(from_idx) = next else {
            break;
        };
        processed[from_idx] = true;

        let from_path = nodes[from_idx].path.clone();
        let from_dir = from_path.parent().unwrap_or(Path::new("")).to_path_buf();
        let Some(from_parsed) = available.get(&from_path) else {
            continue;
        };
        let from_ast = &from_parsed.output.ast;
        let spans = &from_ast.span_table;

        for import in &from_ast.uses {
            for module_import in &import.0 {
                let span_id = HasSpanId::span_id(module_import);
                let mut import_symptoms: Vec<Diagnostic> = Vec::new();
                let resolved = resolve_module_import(
                    module_import,
                    &from_dir,
                    deps,
                    spans,
                    span_id,
                    &mut import_symptoms,
                    find_public_def,
                );

                for s in import_symptoms {
                    symptoms.push((from_idx, s));
                }

                node_aliases[from_idx].extend(resolved.symbol_aliases);

                let from_folder = from_path.parent().map(Path::to_path_buf);
                for (file_path, qual) in resolved.files {
                    if available.get(&file_path).is_none() {
                        symptoms.push((
                            from_idx,
                            Diagnostic::new(
                                "use-target-not-found",
                                format!("import target not found: `{}`", file_path.display()),
                            )
                            .with_help("ensure the import path points to an existing `.gin` file or folder module")
                            .with_arg("path", file_path.display().to_string())
                            .at_span(spans.get(span_id)),
                        ));
                        continue;
                    }

                    if let Some(prev) = seen.get(&file_path)
                        && prev != &qual
                    {
                        symptoms.push((
                            from_idx,
                            Diagnostic::new(
                                "use-conflict",
                                format!(
                                    "import conflict: {} is pulled in as `{}` and `{}`",
                                    file_path.display(),
                                    prev,
                                    qual,
                                ),
                            )
                            .with_help("choose a single qualifier/alias for this module")
                            .with_arg("path", file_path.display().to_string())
                            .with_arg("qualifier_a", prev.clone())
                            .with_arg("qualifier_b", qual)
                            .at_span(spans.get(span_id)),
                        ));
                        continue;
                    }

                    let to_idx = if let Some(i) = node_by_path.get(&file_path).copied() {
                        i
                    } else {
                        let i = nodes.len();
                        nodes.push(ResolveNode {
                            path: file_path.clone(),
                            qualifier: qual.clone(),
                        });
                        adj.push(Vec::new());
                        node_aliases.push(Vec::new());
                        processed.push(false);
                        seen.insert(file_path.clone(), qual.clone());
                        node_by_path.insert(file_path.clone(), i);
                        i
                    };

                    let cross_folder = from_folder.as_deref() != file_path.parent();
                    if cross_folder {
                        adj[from_idx].push(ImportEdge {
                            to: to_idx,
                            import_span: span_id,
                        });
                    }
                }
            }
        }
    }

    if let Some(cycle) = detect_first_cycle(&adj) {
        let mut parts: Vec<String> = Vec::new();
        for &n in &cycle.nodes {
            parts.push(nodes[n].path.display().to_string());
        }
        let chain = parts.join(" -> ");
        let cycle_span = available
            .get(&nodes[cycle.closing_from].path)
            .map(|f| f.output.ast.span_table.get(cycle.closing_span))
            .unwrap_or(Span::new(0, 0));
        symptoms.push((
            cycle.closing_from,
            Diagnostic::new("use-cycle", "import cycle detected")
                .with_help(format!("cycle: {chain}"))
                .at_span(cycle_span),
        ));
    }

    ResolveGraph {
        nodes,
        adj,
        node_aliases,
        symptoms,
    }
}

#[cfg(test)]
mod tests {
    use super::{PublicSymbolIndex, discovery};
    use crate::ParsedFile;
    use parser::query::SourceParseExt;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn parsed_file(path: &Path) -> ParsedFile {
        let source = fs::read_to_string(path).unwrap();
        let output = source.parse_source_full();
        ParsedFile {
            path: path.to_path_buf(),
            source,
            output,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("gin_resolve_graph_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn discovery_uses_parsed_public_symbol_index_after_dependency_source_is_removed() {
        let root = temp_dir("parsed_symbol_index");
        let core_dir = root.join("core");
        fs::create_dir_all(&core_dir).unwrap();
        fs::write(core_dir.join("flask.jsonc"), "{}").unwrap();

        let main_path = root.join("main.gin");
        let int_path = core_dir.join("int.gin");
        fs::write(&main_path, "use core.Int\n").unwrap();
        fs::write(&int_path, "Int is Unit\n").unwrap();

        let main = parsed_file(&main_path);
        let int = parsed_file(&int_path);
        let mut available = HashMap::new();
        available.insert(main_path.clone(), main);
        available.insert(int_path.clone(), int);
        let public_symbols = PublicSymbolIndex::from_parsed_files(&available);
        fs::remove_file(&int_path).unwrap();

        let mut dependencies = HashMap::new();
        dependencies.insert("core".to_string(), core_dir);
        let graph = discovery(&available, &[main_path], &dependencies, &|dir, symbol| {
            public_symbols.find(dir, symbol)
        });

        assert!(graph.symptoms.is_empty(), "{:#?}", graph.symptoms);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn build_import_closure_skips_dependencies_without_entry_imports() {
        let root = temp_dir("unused_dependency");
        let dependency_dir = root.join("dependency");
        fs::create_dir_all(&dependency_dir).unwrap();
        fs::write(dependency_dir.join("unused.gin"), "this is not valid gin\n").unwrap();

        let main_path = root.join("main.gin");
        fs::write(&main_path, "main:\nreturn\n").unwrap();
        let mut dependencies = HashMap::new();
        dependencies.insert("dependency".to_string(), dependency_dir);

        let (_graph, available) =
            super::build_import_closure(vec![parsed_file(&main_path)], &dependencies);

        assert_eq!(available.len(), 1);
        assert!(available.contains_key(&main_path));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_symbol_index_excludes_private_definitions() {
        let root = temp_dir("private_symbol");
        let file_path = root.join("private.gin");
        fs::write(&file_path, "private\nHidden is Unit\n").unwrap();

        let mut files = HashMap::new();
        files.insert(file_path.clone(), parsed_file(&file_path));
        let public_symbols = PublicSymbolIndex::from_parsed_files(&files);

        assert!(public_symbols.find(&root, "Hidden").is_none());
        let _ = fs::remove_dir_all(root);
    }
}
