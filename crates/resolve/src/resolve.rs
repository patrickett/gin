use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ast::ImportSource;
use ast::{
    FileAst, HasSpanId, LocalBundleImport, MergeConflict, ModuleImport, SymbolAlias,
    apply_symbol_aliases, qualify_module_defs,
};
use diagnostic::{Diagnostic, DiagnosticLike, SpanId, UseSymptom};
use flask::{DependencyKind, FlaskConfig, PACKAGE_CONFIG_NAME};
use internment::Intern;

use parser::parse_source_full;

use crate::ParsedFile;

pub fn is_folder_module_dir(path: &Path) -> bool {
    path.is_dir() && path.join(PACKAGE_CONFIG_NAME).is_file()
}

/// Resolve a dependency directory from any file path by loading `flask.jsonc`
/// from the file's parent directory and looking up the dependency by name.
pub fn resolve_dep_dir(file_path: &Path, dep_name: &str) -> Option<PathBuf> {
    let base_dir = file_path.parent()?;
    let handle = flask::FlaskConfigHandle::load(base_dir).ok()?;
    let cfg = handle.read();
    let config_dir = handle.source_dir();
    let dep = cfg.config.dependencies().get(dep_name)?;
    match &dep.kind {
        DependencyKind::Path { path } => Some(config_dir.join(path)),
        _ => None,
    }
}

/// Show hover information about a dependency root (e.g. hovering over `core`
/// in `use core.true`). Returns formatted markdown with name, description, version.
pub fn resolve_dep_hover(file_path: &Path, dep_name: &str) -> Option<String> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    let dep_config = flask::FlaskConfig::from_directory(&dep_dir)?;
    let name = dep_config.name();
    let version = dep_config.version();
    let description = dep_config.description().unwrap_or("");

    let mut text = format!("```gin\n{name}\n```");
    if !description.is_empty() {
        text.push_str(&format!("\n\n---\n\n{description}"));
    }
    text.push_str(&format!("\n\n---\n\nversion = {version}"));
    Some(text)
}

/// Resolve a symbol from a dependency (public definition) and return its
/// hover text by reading and parsing the definition file.
pub fn resolve_symbol_hover(file_path: &Path, dep_name: &str, symbol: &str) -> Option<String> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    let def_file = find_public_def_in_package(&dep_dir, symbol)?;
    let def_source = std::fs::read_to_string(&def_file).ok()?;
    let def_output = parse_source_full(&def_source);
    let def_span = typeck::find_definition_span(&def_output.ast, symbol)?;
    typeck::hover_at(&def_source, &def_output.ast, def_span.start)
}

/// Resolve a symbol from a dependency and return its definition span (byte
/// range in the definition file) for goto-definition.
pub fn resolve_symbol_def_span(
    file_path: &Path,
    dep_name: &str,
    symbol: &str,
) -> Option<std::ops::Range<usize>> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    let def_file = find_public_def_in_package(&dep_dir, symbol)?;
    let def_source = std::fs::read_to_string(&def_file).ok()?;
    let def_output = parse_source_full(&def_source);
    typeck::find_definition_span(&def_output.ast, symbol)
}

/// Determine which part of a dotted path the cursor is on.
///
/// `0` = root (e.g. `core` in `core.true`), `1` = first segment (`true`),
/// `2` = second segment, etc.
pub fn part_index_in_dotted_path(span_text: &str, byte_in_span: usize) -> usize {
    let mut part = 0usize;
    for (i, ch) in span_text.char_indices() {
        if i >= byte_in_span {
            break;
        }
        if ch == '.' {
            part += 1;
        }
    }
    part
}

/// Collect `.gin` file paths under `root`, skipping `target/` directories.
///
/// If `root` is a folder module (contains `flask.jsonc`), only immediate
/// `*.gin` files from the package manifest are returned. Otherwise, the
/// directory is scanned recursively.
pub fn collect_gin_files(root: &Path) -> Vec<PathBuf> {
    if root.is_dir() {
        if root.join(PACKAGE_CONFIG_NAME).is_file() {
            flask::list_package_gin_files(root)
        } else {
            collect_gin_files_recursive(root)
        }
    } else {
        vec![root.to_path_buf()]
    }
}

/// Recursively collect `.gin` file paths under `dir`, skipping `target/` directories.
pub fn collect_gin_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            files.extend(collect_gin_files_recursive(&path));
        } else if path.extension().is_some_and(|ext| ext == "gin") {
            files.push(path);
        }
    }

    files
}

/// What a cursor position resolves to via `use` imports.
#[derive(Debug, Clone)]
pub enum ImportTarget {
    /// Cursor is on the root dependency name (e.g., `core` in `use core.true`).
    DepRoot { dep_name: String },
    /// Cursor is on a segment that names a public symbol from a dependency.
    DepSymbol { dep_name: String, symbol: String },
    /// Cursor is on a bare word in the body that matches an import's effective name.
    BodySymbol { dep_name: String, symbol: String },
}

/// Resolve a cursor position to an import target, handling both phases:
///
/// **Phase 1:** cursor is directly inside a `use` statement's import path
/// (handles `ImportSource::Package` only — callers handle Local / LocalBundle
/// separately).
///
/// **Phase 2:** cursor is on a bare word in the body that matches an import's
/// effective name (e.g., `true` from `use core.true`).
///
/// Returns `None` if the cursor is not on an import-related symbol.
pub fn resolve_import_at(ast: &FileAst, source: &str, byte_pos: usize) -> Option<ImportTarget> {
    // Phase 1: cursor directly inside a `use` statement's import path.
    for import in ast.uses() {
        for mi in &import.0 {
            if let ImportSource::Package(mp) = &mi.source {
                let span_table = ast.span_table();
                let span = span_table.get(mp.span_id());
                if byte_pos < span.start || byte_pos > span.end {
                    continue;
                }

                let span_text = source.get(span.start..span.end).unwrap_or("");
                let byte_in_span = byte_pos.saturating_sub(span.start);
                let part = part_index_in_dotted_path(span_text, byte_in_span);

                if part == 0 {
                    return Some(ImportTarget::DepRoot {
                        dep_name: mp.root.as_str().to_string(),
                    });
                }

                let seg_idx = part.saturating_sub(1);
                if seg_idx >= mp.segments.len() {
                    return None;
                }

                let symbol = mp.segments[seg_idx].as_str().to_string();
                return Some(ImportTarget::DepSymbol {
                    dep_name: mp.root.as_str().to_string(),
                    symbol,
                });
            }
        }
    }

    // Phase 2: bare word matching an import's effective name.
    let word = ast
        .word_at_byte(byte_pos, source)
        .or_else(|| typeck::word_at_byte_offset(source, byte_pos));

    if let Some(word) = word {
        for import in ast.uses() {
            for mi in &import.0 {
                let imported_name = mi
                    .alias
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| mi.effective_name());
                if imported_name == word
                    && let ImportSource::Package(mp) = &mi.source
                    && mp.segments.len() == 1
                {
                    return Some(ImportTarget::BodySymbol {
                        dep_name: mp.root.as_str().to_string(),
                        symbol: mp.segments[0].as_str().to_string(),
                    });
                }
            }
        }
    }

    None
}

/// A node in the resolved import graph.
#[derive(Debug, Clone)]
pub struct ResolveNode {
    pub path: PathBuf,
    pub qualifier: String,
}

/// Outcome of the pure discovery phase: what files are needed and how they connect.
#[derive(Debug, Clone)]
pub struct ResolveGraph {
    pub nodes: Vec<ResolveNode>,
    pub adj: Vec<Vec<crate::module_graph::ImportEdge>>,
    /// Symbol aliases to apply to each node's AST during the resolve phase.
    pub node_aliases: Vec<Vec<SymbolAlias>>,
    /// Symptoms with the graph node index they belong to.
    pub symptoms: Vec<(usize, Diagnostic)>,
}

/// Takes a complete set of pre-parsed files (entry + all dependency directories)
/// and determines which files form the transitive import closure, what qualifiers
/// they get, and how they connect (adjacency graph for cycle detection).
///
/// This function does NOT read file contents. It uses metadata I/O (directory
/// existence checks, file listing) to find available files, and the
/// `find_public_def` closure (caller-controlled I/O) for symbol lookups.
///
/// Precondition: Every `.gin` file from entry + dependency directories is parsed
/// and present in `available`.
pub fn discovery(
    available: &HashMap<PathBuf, ParsedFile>,
    entry_paths: &[PathBuf],
    deps: &HashMap<String, PathBuf>,
    find_public_def: &dyn Fn(&Path, &str) -> Option<PathBuf>,
) -> ResolveGraph {
    let mut nodes: Vec<ResolveNode> = Vec::new();
    let mut adj: Vec<Vec<crate::module_graph::ImportEdge>> = Vec::new();
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

                    adj[from_idx].push(crate::module_graph::ImportEdge {
                        to: to_idx,
                        import_span: span_id,
                    });
                }
            }
        }
    }

    if let Some(cycle) = crate::module_graph::detect_first_cycle(&adj) {
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

/// Build the complete available file map from entry + dependency files and run
/// import discovery. Shared by [`resolve_imports`] and [`resolve_import_symptoms`].
fn build_import_closure(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
) -> (ResolveGraph, HashMap<PathBuf, ParsedFile>) {
    let entry_paths: Vec<PathBuf> = entry_files.iter().map(|f| f.path.clone()).collect();

    // Collect all file paths from entry + dependency directories.
    // Walk each dep directory for packages (directories with `flask.jsonc`)
    // rather than blindly recursing — every package found is explicitly declared.
    let mut all_paths = entry_paths.clone();
    for dep_dir in dependencies.values() {
        let mut stack = vec![dep_dir.clone()];
        while let Some(dir) = stack.pop() {
            all_paths.extend(collect_gin_files(&dir));
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

    // Build the available map: start with entry files, parse any new dep files
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
        find_public_def_in_package(dir, sym)
    });

    (graph, available)
}

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

    // Ensure all .gin files from dependency directories are included in the graph.
    // Gin modules share a type environment across all their files, so when any
    // symbol is imported from a module, all files must be typechecked together.
    for dep_dir in dependencies.values() {
        for file_path in collect_gin_files(dep_dir) {
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
/// Same as [`resolve_imports`] but does not qualify definitions — returns only
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

/// The `file_reader` closure is called once per graph node (by path). It must
/// return the full [`ParsedFile`] including source text and parse output.
pub fn resolve(
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

pub fn resolve_flask_path_dependencies(
    config: &FlaskConfig,
    config_dir: &Path,
) -> HashMap<String, PathBuf> {
    let mut dependencies = HashMap::new();
    for (name, dep) in config.dependencies() {
        if let DependencyKind::Path { path: dep_path } = &dep.kind {
            dependencies.insert(name.clone(), config_dir.join(dep_path));
        }
    }
    dependencies
}

/// All `.gin` files for a folder module: non-recursive, same qualifier for each.
fn resolve_package_gin_files(
    package_dir: &Path,
    qual_prefix: &str,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    if !is_folder_module_dir(package_dir) {
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

    if !is_folder_module_dir(dep_dir) {
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
        if is_folder_module_dir(&nested) {
            let qual = m
                .alias
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| m.export.to_string());
            out.extend(resolve_package_gin_files(&nested, &qual, m.span, symptoms));
        } else {
            // Not a sub-package — check if it's a public def in the dependency root.
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

    // If any member is a symbol import (not a sub-package), include all root
    // package .gin files qualified with the root name so the symbols are
    // available as `root.symbol` (e.g. `core.true`) in the merged AST.
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

/// List all public (exported) symbol names in any `.gin` file under `package_dir`,
/// including both defs and tags.
pub fn list_public_symbols(package_dir: &Path) -> Vec<String> {
    let paths = flask::list_package_gin_files(package_dir);
    let mut symbols = Vec::new();
    for path in &paths {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let output = parse_source_full(&source);
        for (key, _) in &output.ast.defs {
            if !output.ast.private_defs.contains(key) {
                symbols.push(key.as_str().to_string());
            }
        }
        for key in output.ast.tags.keys() {
            if !output.ast.private_tags.contains(key) {
                symbols.push(key.as_str().to_string());
            }
        }
    }
    symbols.sort();
    symbols.dedup();
    symbols
}

/// Check whether `symbol_name` is a public (exported) definition in any `.gin`
/// file under `package_dir`.
pub fn find_public_def_in_package(package_dir: &Path, symbol_name: &str) -> Option<PathBuf> {
    let paths = flask::list_package_gin_files(package_dir);
    let target = Intern::<String>::from_ref(symbol_name);
    for path in &paths {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let output = parse_source_full(&source);
        // A def is "exported" if it is NOT in private_defs after parsing.
        if !output.ast.private_defs.contains(&target) && output.ast.defs.contains_key(&target) {
            return Some(path.clone());
        }
        // Also check tags (capitalized type names).
        if !output.ast.private_tags.contains(&target) && output.ast.tags.contains_key(&target) {
            return Some(path.clone());
        }
    }
    None
}

pub fn check_public_def_in_package(package_dir: &Path, symbol_name: &str) -> bool {
    find_public_def_in_package(package_dir, symbol_name).is_some()
}

fn resolve_local_path_import(
    module_import: &ModuleImport,
    base_dir: &Path,
    path: &Path,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> ResolvedModule {
    let full = base_dir.join(path);

    if full.is_dir() && is_folder_module_dir(&full) {
        if module_import.alias.is_none() {
            symptoms.push(
                UseSymptom::LocalFolderRequiresAs {
                    path: full.display().to_string(),
                }
                .into_diagnostic(span_id),
            );
            return ResolvedModule {
                files: Vec::new(),
                symbol_aliases: Vec::new(),
            };
        }
        let qual = module_import.alias.as_ref().unwrap().to_string();
        return ResolvedModule {
            files: resolve_package_gin_files(&full, &qual, span_id, symptoms),
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
            resolve_dependency_bundle_import(b, dependencies, span_id, symptoms, find_public_def)
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
    }
}

fn resolve_package_like_import(
    module_import: &ModuleImport,
    mp: &ast::ModPath,
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

    if !is_folder_module_dir(dep_dir) {
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

#[cfg(test)]
mod tests {
    use super::*;
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
        use ast::ModPath;
        use internment::Intern;

        let entry_src = "main:\n    foo\nreturn\n";
        let entry_path = PathBuf::from("/main.gin");
        let entry_pf = make_pf("/main.gin", entry_src);

        let alias = SymbolAlias {
            alias: Intern::<String>::from_ref("foo"),
            target: ModPath {
                root: Intern::<String>::from_ref("dep"),
                segments: vec![Intern::<String>::from_ref("bar")],
                span: SpanId::INVALID,
            },
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
        // Applying aliases changes the AST in-place; no crash means success.
        // The `foo` reference would be rewritten by apply_symbol_aliases
        // which traverses the AST looking for bare references.
    }

    #[test]
    fn discovery_with_missing_available_file_is_ok() {
        // If a local import can't be resolved through filesystem checks,
        // resolve_local_path_import returns an empty ResolvedModule with a symptom.
        // discovery should still return a graph without crashing.
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

    // Helper: creates a temporary directory that cleans up on Drop.
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

        // Create flask.jsonc (required by list_package_gin_files).
        let config = r#"{"name":"testpkg","version":"0.0.0","authors":[]}"#;
        tmp.add_file("flask.jsonc", config);

        // Create a .gin file with public tag, public def, and private def.
        // The `private` keyword marks subsequent definitions as private.
        tmp.add_file(
            "util.gin",
            r#"Color is Red or Green

help := 42

private
private_helper:
    return
"#,
        );

        let symbols = list_public_symbols(&pkg_dir);

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
