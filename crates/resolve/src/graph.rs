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

use ast::{HasSpanId, SymbolAlias};
use diagnostic::{Diagnostic, Span};

use crate::ParsedFile;

use crate::module_graph::{ImportEdge, detect_first_cycle};
use crate::module_inventory::ModuleInventory;
use crate::module_loader::{ModuleLoader, ParsedModuleCache};
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

/// Collect all `.gin` files for a package, parse any that aren't already
/// parsed, discover their imports, and build the [`ResolveGraph`].
pub(crate) fn build_import_closure(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
) -> (ResolveGraph, HashMap<PathBuf, ParsedFile>) {
    let (graph, cache) =
        build_import_closure_with_cache(entry_files, dependencies, ParsedModuleCache::default());
    (graph, cache.files)
}

pub(crate) fn build_import_closure_with_cache(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
    cache: ParsedModuleCache,
) -> (ResolveGraph, ParsedModuleCache) {
    let entry_paths: Vec<PathBuf> = entry_files.iter().map(|f| f.path.clone()).collect();
    let inventory = ModuleInventory::discover(dependencies);
    let mut loader = ModuleLoader::with_cache(inventory, entry_files, cache);
    let graph = discovery(&mut loader, &entry_paths, dependencies);

    (graph, loader.into_cache())
}

/// Discover imports and build the full dependency graph.
///
/// Processes nodes iteratively, resolving each file's imports and adding
/// newly discovered files to the graph. Detects cycles and duplicate qualifiers.
pub(crate) fn discovery(
    loader: &mut ModuleLoader,
    entry_paths: &[PathBuf],
    deps: &HashMap<String, PathBuf>,
) -> ResolveGraph {
    let mut nodes: Vec<ResolveNode> = Vec::new();
    let mut adj: Vec<Vec<ImportEdge>> = Vec::new();
    let mut node_aliases: Vec<Vec<SymbolAlias>> = Vec::new();
    let mut symptoms: Vec<(usize, Diagnostic)> = Vec::new();
    let entry_path_set: HashSet<&Path> = entry_paths.iter().map(PathBuf::as_path).collect();
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
        let Some(from_parsed) = loader.parsed_file_cloned(&from_path) else {
            continue;
        };
        let spans = &from_parsed.output.ast.span_table;

        for import in &from_parsed.output.ast.uses {
            for module_import in &import.0 {
                let span_id = HasSpanId::span_id(module_import);
                let mut import_symptoms: Vec<Diagnostic> = Vec::new();
                let resolved = resolve_module_import(
                    module_import,
                    &from_dir,
                    loader,
                    deps,
                    spans,
                    span_id,
                    &mut import_symptoms,
                );

                for s in import_symptoms {
                    symptoms.push((from_idx, s));
                }

                node_aliases[from_idx].extend(resolved.symbol_aliases);

                let from_folder = from_path.parent().map(Path::to_path_buf);
                for (file_path, qual) in resolved.files {
                    loader.load_containing_module(&file_path);
                    if loader.parsed_file(&file_path).is_none() {
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
                        && !entry_path_set.contains(file_path.as_path())
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
        let cycle_span = loader
            .parsed_file(&nodes[cycle.closing_from].path)
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
    use super::discovery;
    use crate::ParsedFile;
    use crate::module_inventory::ModuleInventory;
    use crate::module_loader::ModuleLoader;
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
    fn discovery_uses_loader_cached_symbol_after_dependency_source_is_removed() {
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
        let mut dependencies = HashMap::new();
        dependencies.insert("core".to_string(), core_dir);
        let inventory = ModuleInventory::discover(&dependencies);
        let mut loader = ModuleLoader::new(inventory, [main, int]);
        fs::remove_file(&int_path).unwrap();
        let graph = discovery(&mut loader, &[main_path], &dependencies);

        assert!(graph.symptoms.is_empty(), "{:#?}", graph.symptoms);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn self_import_of_package_entry_does_not_conflict_with_its_entry_qualifier() {
        let root = temp_dir("self_import_entry");
        let core_dir = root.join("core");
        let primitive_dir = core_dir.join("primitive");
        fs::create_dir_all(&primitive_dir).unwrap();
        fs::write(core_dir.join("flask.jsonc"), "{}").unwrap();

        let marker_path = core_dir.join("marker.gin");
        let bool_path = primitive_dir.join("bool.gin");
        fs::write(&marker_path, "use core.primitive.Bool\n").unwrap();
        fs::write(&bool_path, "Bool is True or False\n").unwrap();

        let mut dependencies = HashMap::new();
        dependencies.insert("core".to_string(), core_dir.clone());
        let inventory = ModuleInventory::discover(&dependencies);
        let mut loader = ModuleLoader::new(
            inventory,
            [parsed_file(&marker_path), parsed_file(&bool_path)],
        );
        let graph = discovery(
            &mut loader,
            &[marker_path, bool_path.clone()],
            &dependencies,
        );

        assert!(graph.symptoms.is_empty(), "{:#?}", graph.symptoms);
        let bool_node = graph
            .nodes
            .iter()
            .find(|node| node.path == bool_path)
            .unwrap();
        assert!(bool_node.qualifier.is_empty());
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
    fn module_loader_excludes_private_definitions() {
        let root = temp_dir("private_symbol");
        let file_path = root.join("private.gin");
        fs::write(&file_path, "private\nHidden is Unit\n").unwrap();

        let mut dependencies = HashMap::new();
        dependencies.insert("private".to_string(), root.clone());
        let inventory = ModuleInventory::discover(&dependencies);
        let mut loader = ModuleLoader::new(inventory, []);

        assert!(loader.find_public_def(&root, "Hidden").is_none());
        let _ = fs::remove_dir_all(root);
    }
}
