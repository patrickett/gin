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
use crate::resolved_package_graph::{ResolvedPackageGraph, ResolvedPackageId};

/// A single file in the import graph.
#[derive(Debug, Clone)]
pub struct ResolveNode {
    pub path: PathBuf,
    pub qualifier: String,
    pub package: ResolvedPackageId,
}

/// The full import dependency graph for a package.
#[derive(Debug, Clone)]
pub struct ResolveGraph {
    pub nodes: Vec<ResolveNode>,
    pub adj: Vec<Vec<ImportEdge>>,
    pub node_aliases: Vec<Vec<SymbolAlias>>,
    pub symptoms: Vec<(usize, Diagnostic)>,
    pub packages: ResolvedPackageGraph,
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
    let packages = entry_paths
        .first()
        .map(|path| ResolvedPackageGraph::discover(path))
        .unwrap_or_else(|| ResolvedPackageGraph::discover(Path::new(".")));
    let inventory = ModuleInventory::discover(dependencies);
    let mut loader = ModuleLoader::with_cache(inventory, entry_files, cache);
    let graph = discovery_with_packages(&mut loader, &entry_paths, dependencies, packages);

    (graph, loader.into_cache())
}

/// Discover imports and build the full dependency graph.
///
/// Processes nodes iteratively, resolving each file's imports and adding
/// newly discovered files to the graph. Detects cycles and duplicate qualifiers.
fn discovery_with_packages(
    loader: &mut ModuleLoader,
    entry_paths: &[PathBuf],
    deps: &HashMap<String, PathBuf>,
    packages: ResolvedPackageGraph,
) -> ResolveGraph {
    let mut nodes: Vec<ResolveNode> = Vec::new();
    let mut adj: Vec<Vec<ImportEdge>> = Vec::new();
    let mut node_aliases: Vec<Vec<SymbolAlias>> = Vec::new();
    let mut symptoms: Vec<(usize, Diagnostic)> = packages
        .diagnostics
        .iter()
        .cloned()
        .map(|diagnostic| (0, diagnostic))
        .collect();
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
                package: packages.root,
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
        let package_dependencies = packages
            .node(nodes[from_idx].package)
            .resolved_dependency_roots();
        let dependency_roots =
            if packages.node(nodes[from_idx].package).config.name() == "anonymous" {
                deps
            } else {
                &package_dependencies
            };

        for import in &from_parsed.output.ast.uses {
            for module_import in &import.0 {
                let span_id = HasSpanId::span_id(module_import);
                let mut import_symptoms: Vec<Diagnostic> = Vec::new();
                let resolved = resolve_module_import(
                    module_import,
                    &from_dir,
                    loader,
                    dependency_roots,
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
                            package: packages
                                .package_for_import(nodes[from_idx].package, &file_path)
                                .unwrap_or(nodes[from_idx].package),
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
        packages,
    }
}

#[cfg(test)]
#[path = "../tests/graph_tests.rs"]
mod tests;
