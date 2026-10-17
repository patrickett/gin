use diagnostic::code::ImportCode;
use std::collections::HashMap;
use std::path::PathBuf;

use ast::FileAst;
use diagnostic::Diagnostic;

use crate::conditional_imports::{StageFile, contains_imports, select_imports};
use crate::graph::{ResolveGraph, discovery_with_packages};
use crate::module_loader::ModuleLoader;
use crate::resolved_package_graph::ResolvedPackageGraph;

const DISCOVERY_PASSES_MAX: usize = 1024;

pub(crate) fn discover(
    loader: &mut ModuleLoader,
    entry_paths: &[PathBuf],
    dependencies: &HashMap<String, PathBuf>,
    packages: ResolvedPackageGraph,
) -> ResolveGraph {
    let mut graph = discovery_with_packages(
        loader,
        entry_paths,
        dependencies,
        packages.clone(),
        &HashMap::new(),
    );
    let conditional = graph.nodes.iter().any(|node| {
        loader
            .parsed_file(&node.path)
            .is_some_and(|file| contains_imports(&file.output.ast))
    });
    if !conditional {
        return graph;
    }
    for _ in 0..DISCOVERY_PASSES_MAX {
        let mut files = stage_files(&graph, loader);
        let diagnostics = select_imports(&mut files, &flask::CompileTarget::Library);
        let staged: HashMap<PathBuf, FileAst> = files
            .into_iter()
            .map(|file| (file.path, file.ast))
            .collect();
        let mut next =
            discovery_with_packages(loader, entry_paths, dependencies, packages.clone(), &staged);
        if same_nodes(&graph, &next) {
            for (path, diagnostic) in diagnostics {
                if let Some(index) = next.nodes.iter().position(|node| node.path == path) {
                    next.symptoms.push((index, diagnostic));
                }
            }
            return next;
        }
        graph = next;
    }
    graph.symptoms.push((
        0,
        // use-discovery-limit
        Diagnostic::new(
            ImportCode::DiscoveryLimit,
            "conditional imports exceeded the discovery pass limit",
        ),
    ));
    graph
}

fn stage_files(graph: &ResolveGraph, loader: &ModuleLoader) -> Vec<StageFile> {
    let mut files = Vec::with_capacity(graph.nodes.len());
    for (index, node) in graph.nodes.iter().enumerate() {
        let Some(parsed) = loader.parsed_file(&node.path) else {
            continue;
        };
        let mut ast = parsed
            .output
            .ast
            .clone()
            .qualify_module_defs(&node.qualifier);
        ast.symbol_aliases = graph.node_aliases[index].clone();
        ast.apply_symbol_aliases();
        files.push(StageFile {
            path: node.path.clone(),
            ast,
        });
    }
    files
}

fn same_nodes(previous: &ResolveGraph, next: &ResolveGraph) -> bool {
    previous.nodes.len() == next.nodes.len()
        && previous.nodes.iter().enumerate().all(|(index, node)| {
            next.nodes.iter().enumerate().any(|(other_index, other)| {
                node.path == other.path
                    && node.qualifier == other.qualifier
                    && previous.node_aliases[index] == next.node_aliases[other_index]
            })
        })
}
