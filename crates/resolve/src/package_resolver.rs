use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ast::{
    FileAst, HasSpanId, ImportSource, LocalBundleImport, MergeConflict, ModuleImport, SymbolAlias,
    apply_symbol_aliases, qualify_module_defs,
};
use diagnostic::{Diagnostic, DiagnosticLike, SpanId, UseSymptom};

use parser::parse_source_full;

use crate::module_graph::{ImportEdge, detect_first_cycle};
use crate::{ParsedFile, file_helpers};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResolveNode {
    pub path: PathBuf,
    pub qualifier: String,
}

#[derive(Debug, Clone)]
pub struct ResolveGraph {
    pub nodes: Vec<ResolveNode>,
    pub adj: Vec<Vec<ImportEdge>>,
    pub node_aliases: Vec<Vec<SymbolAlias>>,
    pub symptoms: Vec<(usize, Diagnostic)>,
}

// ---------------------------------------------------------------------------
// Public API — the deep seam for the batch pipeline
// ---------------------------------------------------------------------------

/// Full import resolution for binary compilation.
///
/// Collects all `.gin` files from entry + dependency directories, parses any
/// unparsed dependency files, runs import discovery, and qualifies all module
/// definitions from their import paths.
///
/// Use this in the compiler driver (ginc).
pub fn resolve_imports(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
) -> Vec<ParsedFile> {
    let (mut graph, available) = build_import_closure(entry_files, dependencies);

    for dep_dir in dependencies.values() {
        for file_path in file_helpers::collect_gin_files(dep_dir) {
            if !graph.nodes.iter().any(|n| n.path == file_path) {
                let qual = dep_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                graph.nodes.push(ResolveNode {
                    path: file_path,
                    qualifier: qual,
                });
                graph.adj.push(Vec::new());
                graph.node_aliases.push(Vec::new());
            }
        }
    }

    resolve(graph, &mut |path| available.get(path).cloned())
}

/// Import graph discovery for diagnostic collection.
///
/// Same as `resolve_imports` but does not qualify definitions — returns only
/// the import-related symptoms (errors, warnings) grouped by file path.
///
/// Use this in tools that need import diagnostics without qualified ASTs (ginlsp).
pub fn resolve_import_symptoms(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
) -> HashMap<PathBuf, Vec<Diagnostic>> {
    let (graph, _available) = build_import_closure(entry_files, dependencies);
    let mut by_path: HashMap<PathBuf, Vec<Diagnostic>> = HashMap::new();
    for (node_idx, diag) in graph.symptoms {
        if node_idx < graph.nodes.len() {
            by_path
                .entry(graph.nodes[node_idx].path.clone())
                .or_default()
                .push(diag);
        }
    }
    by_path
}

/// Merge compilation units; returns diagnostics for duplicate top-level names.
pub fn merge_asts_checked(files: &[ParsedFile]) -> Result<FileAst, Vec<Diagnostic>> {
    let mut merged = FileAst::default();
    let mut errors = Vec::new();
    for file in files {
        if let Err(conflict) = merged.merge_from_checked(file.output.ast.clone()) {
            let (symbol, span) = match &conflict {
                MergeConflict::Tag { name } => {
                    let span = file
                        .output
                        .ast
                        .tags()
                        .get(name)
                        .map(|t| t.name_span)
                        .unwrap_or(SpanId::INVALID);
                    (name.to_string(), span)
                }
                MergeConflict::Def { name } => {
                    let span = file
                        .output
                        .ast
                        .defs()
                        .get(name)
                        .map(|d| d.name_span)
                        .unwrap_or(SpanId::INVALID);
                    (name.to_string(), span)
                }
            };
            errors.push(UseSymptom::DuplicateTopLevel { symbol }.into_diagnostic(span));
        }
    }
    if errors.is_empty() {
        Ok(merged)
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// Internal (pub(crate) for tests)
// ---------------------------------------------------------------------------

pub(crate) fn build_import_closure(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
) -> (ResolveGraph, HashMap<PathBuf, ParsedFile>) {
    let entry_paths: Vec<PathBuf> = entry_files.iter().map(|f| f.path.clone()).collect();

    let mut all_paths = entry_paths.clone();
    for dep_dir in dependencies.values() {
        let mut stack = vec![dep_dir.clone()];
        while let Some(dir) = stack.pop() {
            all_paths.extend(file_helpers::collect_gin_files(&dir));
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && path.join(flask::PACKAGE_CONFIG_NAME).is_file() {
                        stack.push(path);
                    }
                }
            }
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
            let output = parse_source_full(&source);
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
        file_helpers::find_public_def_in_package(dir, sym)
    });

    (graph, available)
}

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

        let from_path = &nodes[from_idx].path;
        let from_dir = from_path.parent().unwrap_or(Path::new("")).to_path_buf();
        let Some(from_parsed) = available.get(from_path) else {
            continue;
        };
        let from_ast = &from_parsed.output.ast;

        for import in from_ast.uses() {
            for module_import in &import.0 {
                let span_id = HasSpanId::span_id(module_import);
                let mut import_symptoms: Vec<Diagnostic> = Vec::new();
                let resolved = resolve_module_import(
                    module_import,
                    &from_dir,
                    deps,
                    span_id,
                    &mut import_symptoms,
                    find_public_def,
                );

                for s in import_symptoms {
                    symptoms.push((from_idx, s));
                }

                node_aliases[from_idx].extend(resolved.symbol_aliases);

                for (file_path, qual) in resolved.files {
                    if available.get(&file_path).is_none() {
                        symptoms.push((
                            from_idx,
                            UseSymptom::TargetNotFound {
                                path: file_path.display().to_string(),
                            }
                            .into_diagnostic(span_id),
                        ));
                        continue;
                    }

                    if let Some(prev) = seen.get(&file_path)
                        && prev != &qual
                    {
                        symptoms.push((
                            from_idx,
                            UseSymptom::Conflict {
                                path: file_path.display().to_string(),
                                qualifier_a: prev.clone(),
                                qualifier_b: qual,
                            }
                            .into_diagnostic(span_id),
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

                    adj[from_idx].push(ImportEdge {
                        to: to_idx,
                        import_span: span_id,
                    });
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
        symptoms.push((
            cycle.closing_from,
            UseSymptom::Cycle { chain }.into_diagnostic(cycle.closing_span),
        ));
    }

    ResolveGraph {
        nodes,
        adj,
        node_aliases,
        symptoms,
    }
}

pub(crate) fn resolve(
    graph: ResolveGraph,
    file_reader: &mut dyn FnMut(&Path) -> Option<ParsedFile>,
) -> Vec<ParsedFile> {
    let ResolveGraph {
        nodes,
        adj: _adj,
        node_aliases,
        symptoms,
    } = graph;

    let mut files: Vec<Option<ParsedFile>> = Vec::with_capacity(nodes.len());

    for (i, node) in nodes.iter().enumerate() {
        let mut parsed = match file_reader(&node.path) {
            Some(f) => f,
            None => {
                files.push(None);
                continue;
            }
        };

        if !node.qualifier.is_empty() {
            parsed.output.ast = qualify_module_defs(parsed.output.ast, &node.qualifier);
            parsed.output.ast = parsed.output.ast.strip_private_for_importer();
        }

        if !node_aliases[i].is_empty() {
            parsed.output.ast.symbol_aliases = node_aliases[i].clone();
            apply_symbol_aliases(&mut parsed.output.ast);
            parsed.output.ast.symbol_aliases.clear();
        }

        files.push(Some(parsed));
    }

    for (node_idx, diag) in symptoms {
        if let Some(ref mut f) = files[node_idx] {
            f.output.symptoms.push(diag);
        }
    }

    files.into_iter().flatten().collect()
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

struct ResolvedModule {
    files: Vec<(PathBuf, String)>,
    symbol_aliases: Vec<SymbolAlias>,
}

fn resolve_module_import(
    module_import: &ModuleImport,
    base_dir: &Path,
    dependencies: &HashMap<String, PathBuf>,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
    find_public_def: &dyn Fn(&Path, &str) -> Option<PathBuf>,
) -> ResolvedModule {
    match &module_import.source {
        ImportSource::Local(path, _) => {
            resolve_local_path_import(module_import, base_dir, path, span_id, symptoms)
        }
        ImportSource::LocalBundle(b) => {
            if b.local_path.is_some() {
                resolve_local_bundle_import(
                    module_import,
                    b,
                    base_dir,
                    span_id,
                    symptoms,
                    find_public_def,
                )
            } else {
                resolve_dependency_bundle_import(
                    b,
                    dependencies,
                    span_id,
                    symptoms,
                    find_public_def,
                )
            }
        }
        ImportSource::Package(mp) => resolve_package_like_import(
            module_import,
            mp,
            base_dir,
            dependencies,
            span_id,
            symptoms,
            find_public_def,
        ),
        ImportSource::CurrentModule { member } => {
            resolve_current_module_import(member, base_dir, span_id, symptoms, find_public_def)
        }
    }
}

fn resolve_current_module_import(
    member: &ast::BundleExportImport,
    base_dir: &Path,
    _span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
    find_public_def: &dyn Fn(&Path, &str) -> Option<PathBuf>,
) -> ResolvedModule {
    let pkg_root = match crate::import_query::find_package_root(base_dir) {
        Some(r) => r,
        None => {
            return ResolvedModule {
                files: vec![],
                symbol_aliases: vec![],
            };
        }
    };

    let symbol = member.export.as_str();
    if find_public_def(&pkg_root, symbol).is_none() {
        symptoms.push(
            UseSymptom::NotExported {
                symbol: symbol.to_string(),
                module: pkg_root.display().to_string(),
            }
            .into_diagnostic(member.span),
        );
        return ResolvedModule {
            files: vec![],
            symbol_aliases: vec![],
        };
    }

    // Only create a SymbolAlias when there's an actual rename (`use Str as MyStr`)
    let mut symbol_aliases = Vec::new();
    if let Some(alias_name) = member.alias {
        symbol_aliases.push(SymbolAlias {
            alias: alias_name,
            target: ast::Spanned::new(
                ast::ModPath {
                    root: member.export,
                    segments: vec![],
                },
                member.span,
            ),
        });
    }

    ResolvedModule {
        files: vec![],
        symbol_aliases,
    }
}

fn resolve_local_path_import(
    module_import: &ModuleImport,
    base_dir: &Path,
    path: &Path,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> ResolvedModule {
    let full = base_dir.join(path);

    if full.is_dir() {
        let qual = module_import
            .alias
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_else(|| {
                full.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        let gin_files: Vec<(PathBuf, String)> = flask::list_package_gin_files(&full)
            .into_iter()
            .map(|p| (p, qual.clone()))
            .collect();
        if gin_files.is_empty() {
            symptoms.push(
                UseSymptom::PackageHasNoGinFiles {
                    dir: full.display().to_string(),
                }
                .into_diagnostic(span_id),
            );
        }
        return ResolvedModule {
            files: gin_files,
            symbol_aliases: Vec::new(),
        };
    }

    let gin_path = if full.is_file() && full.extension().is_some_and(|e| e == "gin") {
        full.clone()
    } else {
        let with_gin = full.with_extension("gin");
        if with_gin.is_file() {
            with_gin
        } else {
            symptoms.push(
                UseSymptom::LocalNotFound {
                    path: base_dir.join(path).display().to_string(),
                }
                .into_diagnostic(span_id),
            );
            return ResolvedModule {
                files: Vec::new(),
                symbol_aliases: Vec::new(),
            };
        }
    };

    if !gin_path.is_file() {
        symptoms.push(
            UseSymptom::LocalNotFound {
                path: gin_path.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    }

    let stem = gin_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let qual = module_import
        .alias
        .as_ref()
        .map(|a| a.to_string())
        .unwrap_or(stem);
    ResolvedModule {
        files: vec![(gin_path, qual)],
        symbol_aliases: Vec::new(),
    }
}

/// Resolve a local-path bundle import like `use 'arch'.(Constraint, Register, X0)`.
/// Validates that each destructured member is a public symbol in the target module.
fn resolve_local_bundle_import(
    _module_import: &ModuleImport,
    b: &LocalBundleImport,
    base_dir: &Path,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
    find_public_def: &dyn Fn(&Path, &str) -> Option<PathBuf>,
) -> ResolvedModule {
    let local_path = match &b.local_path {
        Some(p) => p,
        None => {
            return ResolvedModule {
                files: vec![],
                symbol_aliases: vec![],
            };
        }
    };

    let full = base_dir.join(local_path);

    if !full.is_dir() {
        symptoms.push(
            UseSymptom::LocalNotFound {
                path: full.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
        return ResolvedModule {
            files: vec![],
            symbol_aliases: vec![],
        };
    }

    let qual = b
        .local_path
        .as_ref()
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed")
        .to_string();

    // Validate each member exists as a public symbol in the target module.
    let mut has_symbol_import = false;
    for member in &b.members {
        let member_name = member.export.as_str();
        if find_public_def(&full, member_name).is_some() {
            has_symbol_import = true;
        } else {
            symptoms.push(
                UseSymptom::NotExported {
                    symbol: member_name.to_string(),
                    module: full.display().to_string(),
                }
                .into_diagnostic(member.span),
            );
        }
    }

    let gin_files: Vec<(PathBuf, String)> = flask::list_package_gin_files(&full)
        .into_iter()
        .map(|p| (p, qual.clone()))
        .collect();

    if has_symbol_import && !gin_files.is_empty() {
        ResolvedModule {
            files: gin_files,
            symbol_aliases: Vec::new(),
        }
    } else {
        ResolvedModule {
            files: vec![],
            symbol_aliases: Vec::new(),
        }
    }
}

fn resolve_dependency_bundle_import(
    b: &LocalBundleImport,
    dependencies: &HashMap<String, PathBuf>,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
    find_public_def: &dyn Fn(&Path, &str) -> Option<PathBuf>,
) -> ResolvedModule {
    let root_name = b.root.as_str();
    let Some(dep_dir) = dependencies.get(root_name) else {
        symptoms.push(
            UseSymptom::UnknownDependency {
                name: root_name.to_string(),
            }
            .into_diagnostic(span_id),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    };

    if !file_helpers::is_folder_module_dir(dep_dir) {
        symptoms.push(
            UseSymptom::DependencyMissingConfig {
                name: root_name.to_string(),
                path: dep_dir.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    }

    let mut out = Vec::new();
    let mut has_symbol_import = false;
    for m in &b.members {
        let nested = dep_dir.join(m.export.as_str());
        if file_helpers::is_folder_module_dir(&nested) {
            let qual = m
                .alias
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| m.export.to_string());
            out.extend(resolve_package_gin_files(&nested, &qual, m.span, symptoms));
        } else {
            let found = find_public_def(dep_dir, m.export.as_str()).is_some();
            if found {
                has_symbol_import = true;
            } else {
                symptoms.push(
                    UseSymptom::NotExported {
                        symbol: m.export.to_string(),
                        module: root_name.to_string(),
                    }
                    .into_diagnostic(m.span),
                );
            }
        }
    }

    if has_symbol_import {
        out.extend(resolve_package_gin_files(
            dep_dir, root_name, span_id, symptoms,
        ));
    }

    ResolvedModule {
        files: out,
        symbol_aliases: Vec::new(),
    }
}

fn resolve_package_like_import(
    module_import: &ModuleImport,
    mp: &ast::Spanned<ast::ModPath>,
    _base_dir: &Path,
    dependencies: &HashMap<String, PathBuf>,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
    find_public_def: &dyn Fn(&Path, &str) -> Option<PathBuf>,
) -> ResolvedModule {
    let root_name = mp.root.as_str();
    let Some(dep_dir) = dependencies.get(root_name) else {
        symptoms.push(
            UseSymptom::UnknownDependency {
                name: root_name.to_string(),
            }
            .into_diagnostic(span_id),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    };

    if !file_helpers::is_folder_module_dir(dep_dir) {
        symptoms.push(
            UseSymptom::DependencyMissingConfig {
                name: root_name.to_string(),
                path: dep_dir.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    }

    if mp.segments.is_empty() {
        let eff_root = module_import
            .alias
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_else(|| root_name.to_string());
        return ResolvedModule {
            files: resolve_package_gin_files(dep_dir, &eff_root, span_id, symptoms),
            symbol_aliases: Vec::new(),
        };
    }

    let segs: Vec<&str> = mp.segments.iter().map(|s| s.as_str()).collect();
    let chain = segs.join(".");
    let eff = match &module_import.alias {
        Some(a) => format!("{a}.{chain}"),
        None => chain,
    };
    match flask::resolve_nested_package_path(dep_dir, &segs) {
        Ok(flask::NestedPackageTarget::FolderModule(dir)) => ResolvedModule {
            files: resolve_package_gin_files(&dir, &eff, span_id, symptoms),
            symbol_aliases: Vec::new(),
        },
        Err(flask::NestedPackageResolveError::NestedPackageNotFound { parent, segment }) => {
            if mp.segments.len() == 1 && segment == mp.segments[0].as_str() {
                let symbol = mp.segments[0];
                if let Some(file_path) = find_public_def(dep_dir, symbol.as_str()) {
                    let alias_name = module_import.alias.unwrap_or(symbol);
                    return ResolvedModule {
                        files: vec![(file_path, root_name.to_string())],
                        symbol_aliases: vec![SymbolAlias {
                            alias: alias_name,
                            target: mp.clone(),
                        }],
                    };
                }
                symptoms.push(
                    UseSymptom::NotExported {
                        symbol: symbol.to_string(),
                        module: root_name.to_string(),
                    }
                    .into_diagnostic(mp.span_id()),
                );
                return ResolvedModule {
                    files: Vec::new(),
                    symbol_aliases: Vec::new(),
                };
            }
            symptoms.push(
                UseSymptom::NestedPackageNotFound {
                    parent: parent.display().to_string(),
                    segment,
                }
                .into_diagnostic(span_id),
            );
            ResolvedModule {
                files: Vec::new(),
                symbol_aliases: Vec::new(),
            }
        }
        Err(flask::NestedPackageResolveError::MissingConfig { dir }) => {
            symptoms.push(
                UseSymptom::MissingConfig {
                    dir: dir.display().to_string(),
                }
                .into_diagnostic(span_id),
            );
            ResolvedModule {
                files: Vec::new(),
                symbol_aliases: Vec::new(),
            }
        }
        Err(flask::NestedPackageResolveError::IntermediateNotFolderModule { path }) => {
            symptoms.push(
                UseSymptom::ChainedExportNotFolder {
                    path: path.display().to_string(),
                }
                .into_diagnostic(span_id),
            );
            ResolvedModule {
                files: Vec::new(),
                symbol_aliases: Vec::new(),
            }
        }
    }
}

/// All `.gin` files for a folder module: non-recursive, same qualifier for each.
fn resolve_package_gin_files(
    package_dir: &Path,
    qual_prefix: &str,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    if !file_helpers::is_folder_module_dir(package_dir) {
        symptoms.push(
            UseSymptom::FolderMissingConfig {
                folder: package_dir.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
        return Vec::new();
    }

    let paths = flask::list_package_gin_files(package_dir);
    if paths.is_empty() {
        symptoms.push(
            UseSymptom::PackageHasNoGinFiles {
                dir: package_dir.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
    }

    paths
        .into_iter()
        .map(|p| (p, qual_prefix.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use internment::Intern;
    use parser::parse_source_full;

    fn make_pf(path: &str, source: &str) -> ParsedFile {
        let output = parse_source_full(source);
        ParsedFile {
            path: PathBuf::from(path),
            source: source.to_string(),
            output,
        }
    }

    #[test]
    fn discovery_no_imports_returns_single_node() {
        let pf = make_pf("main.gin", "x := 42\n");
        let mut available = HashMap::new();
        available.insert(PathBuf::from("main.gin"), pf);

        let graph = discovery(
            &available,
            &[PathBuf::from("main.gin")],
            &HashMap::new(),
            &|_, _| None,
        );

        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].path, PathBuf::from("main.gin"));
        assert!(graph.nodes[0].qualifier.is_empty());
        assert!(graph.symptoms.is_empty());
    }

    #[test]
    fn resolve_qualifies_imported_files() {
        let dep_src = "x := 42\n";
        let dep_path = PathBuf::from("/dep.gin");
        let entry_path = PathBuf::from("/main.gin");

        let dep_pf = make_pf("/dep.gin", dep_src);
        let entry_pf = make_pf("/main.gin", "main:\n    1\nreturn\n");

        let graph = ResolveGraph {
            nodes: vec![
                ResolveNode {
                    path: entry_path,
                    qualifier: String::new(),
                },
                ResolveNode {
                    path: dep_path.clone(),
                    qualifier: "mydep".to_string(),
                },
            ],
            adj: vec![vec![], vec![]],
            node_aliases: vec![vec![], vec![]],
            symptoms: vec![],
        };

        let files = resolve(graph, &mut |path| {
            if path == "/dep.gin" {
                Some(dep_pf.clone())
            } else if path == "/main.gin" {
                Some(entry_pf.clone())
            } else {
                None
            }
        });

        assert_eq!(files.len(), 2);
        let dep_ast = &files[1].output.ast;
        assert!(
            dep_ast
                .defs
                .contains_key(&Intern::<String>::from_ref("mydep.x")),
            "expected qualified def 'mydep.x'"
        );
    }

    #[test]
    fn resolve_applies_symbol_aliases() {
        use ast::{ModPath, Spanned};

        let entry_src = "main:\n    foo\nreturn\n";
        let entry_path = PathBuf::from("/main.gin");
        let entry_pf = make_pf("/main.gin", entry_src);

        let alias = SymbolAlias {
            alias: Intern::<String>::from_ref("foo"),
            target: Spanned::new(
                ModPath {
                    root: Intern::<String>::from_ref("dep"),
                    segments: vec![Intern::<String>::from_ref("bar")],
                },
                SpanId::INVALID,
            ),
        };

        let graph = ResolveGraph {
            nodes: vec![ResolveNode {
                path: entry_path,
                qualifier: String::new(),
            }],
            adj: vec![vec![]],
            node_aliases: vec![vec![alias]],
            symptoms: vec![],
        };

        let files = resolve(graph, &mut |path| {
            if path == "/main.gin" {
                Some(entry_pf.clone())
            } else {
                None
            }
        });

        assert_eq!(files.len(), 1);
    }

    #[test]
    fn discovery_with_missing_available_file_is_ok() {
        let pf = make_pf("main.gin", "use './nonexistent' as foo\n\nx := 42\n");
        let mut available = HashMap::new();
        available.insert(PathBuf::from("main.gin"), pf);

        let graph = discovery(
            &available,
            &[PathBuf::from("main.gin")],
            &HashMap::new(),
            &|_, _| None,
        );

        assert_eq!(graph.nodes.len(), 1);
    }

    #[test]
    fn resolve_with_empty_graph_returns_empty_vec() {
        let graph = ResolveGraph {
            nodes: vec![],
            adj: vec![],
            node_aliases: vec![],
            symptoms: vec![],
        };

        let files = resolve(graph, &mut |_| None);
        assert!(files.is_empty());
    }

    #[test]
    fn resolve_attaches_symptoms_to_correct_file() {
        let pf = make_pf("/main.gin", "x := 42\n");
        let mut symptoms = vec![];
        let diag = UseSymptom::DuplicateTopLevel {
            symbol: "x".to_string(),
        }
        .into_diagnostic(SpanId::INVALID);

        symptoms.push((0usize, diag.clone()));

        let graph = ResolveGraph {
            nodes: vec![ResolveNode {
                path: PathBuf::from("/main.gin"),
                qualifier: String::new(),
            }],
            adj: vec![vec![]],
            node_aliases: vec![vec![]],
            symptoms,
        };

        let files = resolve(graph, &mut |path| {
            if path == "/main.gin" {
                Some(pf.clone())
            } else {
                None
            }
        });

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].output.symptoms.len(), 1);
        assert_eq!(files[0].output.symptoms[0].message, diag.message);
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("resolve_test_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn add_file(&self, name: &str, contents: &str) {
            std::fs::write(self.path.join(name), contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn list_public_symbols_returns_defs_and_tags() {
        let tmp = TempDir::new("list_public_symbols");
        let pkg_dir = tmp.path.clone();

        let config = r#"{"name":"testpkg","version":"0.0.0","authors":[]}"#;
        tmp.add_file("flask.jsonc", config);

        tmp.add_file(
            "util.gin",
            r#"Color is Red or Green

help := 42

private
private_helper:
    return
"#,
        );

        let symbols = file_helpers::list_public_symbols(&pkg_dir);

        assert!(
            symbols.contains(&"Color".to_string()),
            "expected 'Color' to be listed as a public tag"
        );
        assert!(
            symbols.contains(&"help".to_string()),
            "expected 'help' to be listed as a public def"
        );
        assert!(
            !symbols.contains(&"private_helper".to_string()),
            "expected 'private_helper' to be excluded as a private def"
        );
    }
}
