//! Import graph construction — build the file dependency graph for a package.
//!
//! This phase collects all `.gin` files (entry + dependencies), parses any
//! unparsed dependency files, discovers their imports, and produces a
//! [`ResolveGraph`] describing the file-level dependency structure.
//!
//! Import resolution (matching import statements to files) is delegated to
//! [`super::resolve_module_import`] and related functions in `package_resolver.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ast::{HasSpanId, SymbolAlias};
use diagnostic::{Diagnostic, Span};

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

/// Collect all `.gin` files for a package, parse any that aren't already
/// parsed, discover their imports, and build the [`ResolveGraph`].
pub(crate) fn build_import_closure(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
) -> (ResolveGraph, HashMap<PathBuf, ParsedFile>) {
    let entry_paths: Vec<PathBuf> = entry_files.iter().map(|f| f.path.clone()).collect();

    let mut all_paths = entry_paths.clone();
    for dep_dir in dependencies.values() {
        all_paths.extend(dep_dir.collect_gin_files());
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

    let graph = discovery(&available, &entry_paths, dependencies, &|dir, sym| {
        dir.find_public_def(sym)
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
