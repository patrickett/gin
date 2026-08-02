use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use ast::{
    BundleExportImport, ImportSource, LocalBundleImport, LocalMemberImport, ModPath, ModuleImport,
    Spanned, SymbolAlias,
};
use diagnostic::{Diagnostic, SpanId, SpanTable};

use crate::ParsedFile;

use crate::folder_module::{
    NormalizePathError, normalize_local_import_path, resolve_logical_module_dir,
    split_dep_path_segments,
};
use crate::graph::{ResolveGraph, build_import_closure, build_import_closure_with_cache};
use crate::module_loader::{ModuleLoader, ParsedModuleCache};
use flask::FlaskPathExt;

#[derive(Debug, Clone, Default)]
pub struct ImportDependencyGraph {
    /// Module or file path → paths that import it (including direct and transitive).
    import_edges: HashMap<PathBuf, Vec<PathBuf>>,
}

impl ImportDependencyGraph {
    fn from_resolve_graph(graph: &ResolveGraph) -> Self {
        let mut import_edges: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

        for node in &graph.nodes {
            import_edges.entry(node.path.clone()).or_default();
        }

        for (from_idx, targets) in graph.adj.iter().enumerate() {
            let Some(from_node) = graph.nodes.get(from_idx) else {
                continue;
            };
            let from_path = from_node.path.clone();

            for edge in targets {
                let Some(to_node) = graph.nodes.get(edge.to) else {
                    continue;
                };
                import_edges
                    .entry(to_node.path.clone())
                    .or_default()
                    .push(from_path.clone());
            }
        }

        Self { import_edges }
    }

    pub fn impacted_modules(&self, path: &Path) -> Vec<PathBuf> {
        if !self.import_edges.contains_key(path) {
            return Vec::new();
        }

        let mut seen: HashMap<PathBuf, bool> = HashMap::new();
        let mut queue = VecDeque::new();
        let mut impacted = Vec::new();

        queue.push_back(path.to_path_buf());
        while let Some(current) = queue.pop_front() {
            if seen.insert(current.clone(), true).is_some() {
                continue;
            }
            impacted.push(current.clone());
            for importer in self.import_edges.get(&current).into_iter().flatten() {
                queue.push_back(importer.clone());
            }
        }

        impacted
    }
}

pub struct ResolveImportsWithGraph {
    pub files: Vec<ParsedFile>,
    pub cache: ParsedModuleCache,
    pub import_graph: ImportDependencyGraph,
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
    let ResolveImportsWithGraph {
        files,
        cache: _,
        import_graph: _,
    } = resolve_imports_with_graph(entry_files, dependencies, ParsedModuleCache::default());
    files
}

pub fn resolve_imports_with_graph(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
    cache: ParsedModuleCache,
) -> ResolveImportsWithGraph {
    let (graph, cache) = build_import_closure_with_cache(entry_files, dependencies, cache);
    let import_graph = ImportDependencyGraph::from_resolve_graph(&graph);
    let files = resolve(graph, &mut |path| cache.files.get(path).cloned());
    ResolveImportsWithGraph {
        files,
        cache,
        import_graph,
    }
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
            parsed.output.ast = parsed.output.ast.qualify_module_defs(&node.qualifier);
            parsed.output.ast = parsed.output.ast.strip_private_for_importer();
        }

        if !node_aliases[i].is_empty() {
            parsed.output.ast.symbol_aliases = node_aliases[i].clone();
            parsed.output.ast.apply_symbol_aliases();
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

pub(crate) struct ResolvedModule {
    pub(crate) files: Vec<(PathBuf, String)>,
    pub(crate) symbol_aliases: Vec<SymbolAlias>,
}

struct ResolveImportEnv<'a> {
    dependencies: &'a HashMap<String, PathBuf>,
    spans: &'a SpanTable,
    span_id: SpanId,
    symptoms: &'a mut Vec<Diagnostic>,
}

struct ResolveLocalPathImportCtx<'a> {
    module_loader: &'a mut ModuleLoader,
    base_dir: &'a Path,
    dependencies: &'a HashMap<String, PathBuf>,
    spans: &'a SpanTable,
    span_id: SpanId,
    symptoms: &'a mut Vec<Diagnostic>,
}

pub(crate) fn resolve_module_import(
    module_import: &ModuleImport,
    base_dir: &Path,
    module_loader: &mut ModuleLoader,
    dependencies: &HashMap<String, PathBuf>,
    spans: &SpanTable,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> ResolvedModule {
    match &module_import.source {
        ImportSource::Local(path, _) => resolve_local_path_import(
            path,
            module_import
                .alias
                .as_ref()
                .map(|alias| alias.as_ref().as_str()),
            &mut ResolveLocalPathImportCtx {
                module_loader,
                base_dir,
                dependencies,
                spans,
                span_id,
                symptoms,
            },
        ),
        ImportSource::LocalBundle(b) => {
            if b.local_path.is_some() {
                resolve_local_bundle_import(
                    module_import,
                    b,
                    module_loader,
                    base_dir,
                    spans,
                    span_id,
                    symptoms,
                )
            } else {
                resolve_dependency_bundle_import(
                    b,
                    module_loader,
                    dependencies,
                    spans,
                    span_id,
                    symptoms,
                )
            }
        }
        ImportSource::Package(mp) => resolve_package_like_import(
            module_import,
            mp,
            module_loader,
            ResolveImportEnv {
                dependencies,
                spans,
                span_id,
                symptoms,
            },
        ),
        ImportSource::CurrentModule { member } => {
            resolve_current_module_import(member, base_dir, module_loader, spans, symptoms)
        }
        ImportSource::LocalMember(m) => {
            resolve_local_member_import(m, base_dir, module_loader, spans, symptoms)
        }
    }
}

fn resolve_current_module_import(
    member: &BundleExportImport,
    file_dir: &Path,
    module_loader: &mut ModuleLoader,
    spans: &SpanTable,
    symptoms: &mut Vec<Diagnostic>,
) -> ResolvedModule {
    let symbol = member.export.as_str();
    if module_loader.find_public_def(file_dir, symbol).is_none() {
        symptoms.push(
            Diagnostic::new(
                "use-not-exported",
                format!("`{}` is not exported from `{}`", symbol, file_dir.display()),
            )
            .with_help(format!(
                "`{symbol}` is not exported from `{}`",
                file_dir.display()
            ))
            .with_arg("symbol", symbol.to_string())
            .with_arg("module", file_dir.display().to_string())
            .at_span(spans.get(member.span)),
        );
        return ResolvedModule {
            files: vec![],
            symbol_aliases: vec![],
        };
    }

    let mut symbol_aliases = Vec::new();
    if let Some(alias_name) = member.alias {
        symbol_aliases.push(member_symbol_alias(member.export, alias_name, member.span));
    }

    ResolvedModule {
        files: vec![],
        symbol_aliases,
    }
}

fn member_symbol_alias(
    export: internment::Intern<String>,
    alias: internment::Intern<String>,
    span: SpanId,
) -> SymbolAlias {
    SymbolAlias {
        alias,
        target: Spanned::new(ModPath::new(export, vec![]), span),
    }
}

fn flat_member_symbol_alias(qual_prefix: &str, member: &BundleExportImport) -> SymbolAlias {
    let alias = member.flat_name();
    let mut parts: Vec<String> = qual_prefix
        .split('.')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    parts.extend(
        member
            .export
            .as_str()
            .split('.')
            .filter(|p| !p.is_empty())
            .map(str::to_string),
    );
    let root = internment::Intern::new(
        parts
            .first()
            .cloned()
            .unwrap_or_else(|| member.export.to_string()),
    );
    let segments = parts
        .iter()
        .skip(1)
        .map(|s| internment::Intern::new(s.clone()))
        .collect();
    SymbolAlias {
        alias,
        target: Spanned::new(ModPath::new(root, segments), member.span),
    }
}

/// Resolve one `use dep.(folder.Symbol, …)` entry against `module_dir`.
fn resolve_bundle_member_at(
    module_loader: &mut ModuleLoader,
    module_dir: &Path,
    qual_prefix: &str,
    member: &BundleExportImport,
    spans: &SpanTable,
    symptoms: &mut Vec<Diagnostic>,
) -> (Vec<(PathBuf, String)>, Option<SymbolAlias>) {
    let export = member.export.as_str();
    let segments: Vec<&str> = export.split('.').filter(|s| !s.is_empty()).collect();

    if segments.is_empty() {
        return (Vec::new(), None);
    }

    if segments.len() == 1 {
        let name = segments[0];
        if let Some(subdir) = resolve_logical_module_dir(module_dir, &[name]) {
            let qual = member
                .alias
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| name.to_string());
            let files = resolve_folder_module_files(
                module_loader,
                &subdir,
                &qual,
                spans,
                member.span,
                symptoms,
            );
            return (files, None);
        }
        if let Some(file_path) = module_loader.find_public_def(module_dir, name) {
            return (
                vec![(file_path, qual_prefix.to_string())],
                Some(flat_member_symbol_alias(qual_prefix, member)),
            );
        }
    } else {
        let (module_segs, trailing_member) = split_dep_path_segments(module_dir, &segments);
        if let Some(sym) = trailing_member {
            let folder = if module_segs.is_empty() {
                module_dir.to_path_buf()
            } else {
                let refs: Vec<&str> = module_segs.iter().map(|s| s.as_str()).collect();
                match resolve_logical_module_dir(module_dir, &refs) {
                    Some(d) => d,
                    None => {
                        symptoms.push(
                            Diagnostic::new(
                                "use-not-exported",
                                format!("`{}` is not exported from `{}`", export, qual_prefix),
                            )
                            .with_help(format!("`{export}` is not exported from `{qual_prefix}`"))
                            .with_arg("symbol", export.to_string())
                            .with_arg("module", qual_prefix.to_string())
                            .at_span(spans.get(member.span)),
                        );
                        return (Vec::new(), None);
                    }
                }
            };
            if let Some(file_path) = module_loader.find_public_def(&folder, &sym) {
                return (
                    vec![(file_path, qual_prefix.to_string())],
                    Some(flat_member_symbol_alias(qual_prefix, member)),
                );
            }
        } else if !module_segs.is_empty() {
            let refs: Vec<&str> = module_segs.iter().map(|s| s.as_str()).collect();
            if let Some(subdir) = resolve_logical_module_dir(module_dir, &refs) {
                let qual = member
                    .alias
                    .as_ref()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| export.to_string());
                let files = resolve_folder_module_files(
                    module_loader,
                    &subdir,
                    &qual,
                    spans,
                    member.span,
                    symptoms,
                );
                return (files, None);
            }
        }
    }

    symptoms.push(
        Diagnostic::new(
            "use-not-exported",
            format!("`{}` is not exported from `{}`", export, qual_prefix),
        )
        .with_help(format!("`{export}` is not exported from `{qual_prefix}`"))
        .with_arg("symbol", export.to_string())
        .with_arg("module", qual_prefix.to_string())
        .at_span(spans.get(member.span)),
    );
    (Vec::new(), None)
}

fn path_to_qual_prefix(path: &Path) -> String {
    path.components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn resolve_local_folder_module(
    base_dir: &Path,
    import_path: &Path,
    spans: &SpanTable,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Option<PathBuf> {
    let pkg_root = crate::import_query::find_package_root(base_dir)?;
    match normalize_local_import_path(base_dir, &pkg_root, import_path) {
        Ok(p) => Some(p),
        Err(NormalizePathError::ImportTargetMustBeFolder { path }) => {
            symptoms.push(
                Diagnostic::new(
                    "use-import-target-must-be-folder",
                    format!(
                        "`{}` must be a folder to import as a folder module",
                        path.display()
                    ),
                )
                .with_help("use `use './file.gin'` for local file imports")
                .at_span(spans.get(span_id)),
            );
            None
        }
        Err(NormalizePathError::EscapesPackageRoot { path, .. }) => {
            symptoms.push(
                Diagnostic::new(
                    "use-escapes-package-root",
                    format!("import path `{}` escapes the package root", path.display()),
                )
                .with_help("ensure the import path stays within the current package")
                .at_span(spans.get(span_id)),
            );
            None
        }
        Err(NormalizePathError::NotADirectory(path)) => {
            symptoms.push(
                Diagnostic::new(
                    "use-local-not-found",
                    format!("local import not found: `{}`", path.display()),
                )
                .with_help("check the path relative to this file, and ensure it ends in `.gin`")
                .at_span(spans.get(span_id)),
            );
            None
        }
    }
}

fn resolve_local_member_import(
    m: &LocalMemberImport,
    base_dir: &Path,
    module_loader: &mut ModuleLoader,
    spans: &SpanTable,
    symptoms: &mut Vec<Diagnostic>,
) -> ResolvedModule {
    let local_path = match &m.local_path {
        Some(p) => p,
        None => {
            return ResolvedModule {
                files: vec![],
                symbol_aliases: vec![],
            };
        }
    };

    let Some(module_dir) =
        resolve_local_folder_module(base_dir, local_path, spans, m.span, symptoms)
    else {
        return ResolvedModule {
            files: vec![],
            symbol_aliases: vec![],
        };
    };

    let symbol = m.member.export.as_str();
    let Some(member_file) = module_loader.find_public_def(&module_dir, symbol) else {
        symptoms.push(
            Diagnostic::new(
                "use-not-exported",
                format!(
                    "`{}` is not exported from `{}`",
                    symbol,
                    module_dir.display()
                ),
            )
            .with_help(format!(
                "`{symbol}` is not exported from `{}`",
                module_dir.display()
            ))
            .with_arg("symbol", symbol.to_string())
            .with_arg("module", module_dir.display().to_string())
            .at_span(spans.get(m.member.span)),
        );
        return ResolvedModule {
            files: vec![],
            symbol_aliases: vec![],
        };
    };

    let qual = path_to_qual_prefix(local_path);
    ResolvedModule {
        files: vec![(member_file, qual.clone())],
        symbol_aliases: vec![flat_member_symbol_alias(&qual, &m.member)],
    }
}

fn resolve_local_path_import(
    path: &Path,
    alias: Option<&str>,
    ctx: &mut ResolveLocalPathImportCtx<'_>,
) -> ResolvedModule {
    let module_loader = &mut *ctx.module_loader;
    let base_dir = ctx.base_dir;
    let dependencies = ctx.dependencies;
    let spans = ctx.spans;
    let span_id = ctx.span_id;
    let symptoms = &mut *ctx.symptoms;

    let Some(alias) = alias else {
        symptoms.push(
            Diagnostic::new(
                "use-local-folder-requires-as",
                format!(
                    "folder module `{}` must be imported with `as` (e.g. `use '{}' as name`)",
                    path.display(),
                    path.display()
                ),
            )
            .with_help("add `as Alias` so the folder module has a single namespace prefix")
            .at_span(spans.get(span_id)),
        );
        return ResolvedModule {
            files: vec![],
            symbol_aliases: vec![],
        };
    };

    let Some(pkg_root) = crate::import_query::find_package_root(base_dir) else {
        return ResolvedModule {
            files: vec![],
            symbol_aliases: vec![],
        };
    };

    let full = match normalize_local_import_path(base_dir, &pkg_root, path) {
        Ok(p) => p,
        Err(NormalizePathError::ImportTargetMustBeFolder { path }) => {
            symptoms.push(
                Diagnostic::new(
                    "use-import-target-must-be-folder",
                    format!(
                        "`{}` must be a folder to import as a folder module",
                        path.display()
                    ),
                )
                .with_help("use `use './file.gin'` for local file imports")
                .at_span(spans.get(span_id)),
            );
            return ResolvedModule {
                files: vec![],
                symbol_aliases: vec![],
            };
        }
        Err(NormalizePathError::EscapesPackageRoot { path, .. }) => {
            symptoms.push(
                Diagnostic::new(
                    "use-escapes-package-root",
                    format!("import path `{}` escapes the package root", path.display()),
                )
                .with_help("ensure the import path stays within the current package")
                .at_span(spans.get(span_id)),
            );
            return ResolvedModule {
                files: vec![],
                symbol_aliases: vec![],
            };
        }
        Err(NormalizePathError::NotADirectory(path)) => {
            symptoms.push(
                Diagnostic::new(
                    "use-local-not-found",
                    format!("local import not found: `{}`", path.display()),
                )
                .with_help("check the path relative to this file, and ensure it ends in `.gin`")
                .at_span(spans.get(span_id)),
            );
            return ResolvedModule {
                files: vec![],
                symbol_aliases: vec![],
            };
        }
    };

    if let Some(dep_name) = path.file_name().and_then(|n| n.to_str()) {
        check_dep_local_collision(
            dep_name,
            alias,
            path,
            dependencies,
            spans,
            span_id,
            symptoms,
        );
    }

    let qual = alias.to_string();
    let gin_files =
        resolve_folder_module_files(module_loader, &full, &qual, spans, span_id, symptoms);
    if gin_files.is_empty() {
        symptoms.push(
            Diagnostic::new(
                "use-package-no-gin-files",
                format!(
                    "folder module `{}` contains no `.gin` source files",
                    full.display()
                ),
            )
            .with_help("add at least one `.gin` file next to flask.jsonc")
            .at_span(spans.get(span_id)),
        );
    }
    ResolvedModule {
        files: gin_files,
        symbol_aliases: Vec::new(),
    }
}

fn check_dep_local_collision(
    name: &str,
    bind_name: &str,
    local_path: &Path,
    dependencies: &HashMap<String, PathBuf>,
    spans: &SpanTable,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) {
    if bind_name == name && dependencies.contains_key(name) {
        symptoms.push(
            Diagnostic::new(
                "use-dep-local-name-collision",
                format!(
                    "local import `{}` uses `{}` which collides with a dependency name `{}`",
                    local_path.display(),
                    bind_name,
                    name,
                ),
            )
            .with_help("rename the local import alias to avoid shadowing the dependency")
            .with_arg("name", name.to_string())
            .with_arg("dependency", name.to_string())
            .with_arg("local_path", local_path.display().to_string())
            .at_span(spans.get(span_id)),
        );
    }
}

/// Resolve a local-path bundle import like `use 'arch'.(Constraint, Register, X0)`.
/// Validates that each destructured member is a public symbol in the target module.
fn resolve_local_bundle_import(
    _module_import: &ModuleImport,
    b: &LocalBundleImport,
    module_loader: &mut ModuleLoader,
    base_dir: &Path,
    spans: &SpanTable,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
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

    let Some(module_dir) =
        resolve_local_folder_module(base_dir, local_path, spans, span_id, symptoms)
    else {
        return ResolvedModule {
            files: vec![],
            symbol_aliases: vec![],
        };
    };

    let qual = path_to_qual_prefix(local_path);
    let mut files = Vec::new();
    let mut symbol_aliases = Vec::new();
    for member in &b.members {
        let (mut member_files, alias) =
            resolve_bundle_member_at(module_loader, &module_dir, &qual, member, spans, symptoms);
        files.append(&mut member_files);
        if let Some(a) = alias {
            symbol_aliases.push(a);
        }
    }

    ResolvedModule {
        files,
        symbol_aliases,
    }
}

fn resolve_dependency_bundle_import(
    b: &LocalBundleImport,
    module_loader: &mut ModuleLoader,
    dependencies: &HashMap<String, PathBuf>,
    spans: &SpanTable,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> ResolvedModule {
    let root_name = b.root.as_str();
    let Some(dep_dir) = dependencies.get(root_name) else {
        symptoms.push(
            Diagnostic::new(
                "use-unknown-dependency",
                format!(
                    "unknown dependency `{}` (not found in flask.jsonc dependencies)",
                    root_name
                ),
            )
            .with_help("add it to `dependencies` in flask.jsonc, or use a local file import")
            .with_arg("name", root_name.to_string())
            .at_span(spans.get(span_id)),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    };

    if !dep_dir.is_folder_module_dir() {
        symptoms.push(
            Diagnostic::new(
                "use-dependency-missing-config",
                format!(
                    "dependency `{}` has no flask.jsonc at {}",
                    root_name,
                    dep_dir.display()
                ),
            )
            .with_help("add a flask.jsonc to the dependency root directory")
            .with_arg("name", root_name.to_string())
            .with_arg("path", dep_dir.display().to_string())
            .at_span(spans.get(span_id)),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    }

    let module_dir = if b.path_segments.is_empty() {
        dep_dir.clone()
    } else {
        let refs: Vec<&str> = b.path_segments.iter().map(|s| s.as_str()).collect();
        match resolve_logical_module_dir(dep_dir, &refs) {
            Some(d) => d,
            None => {
                let segment = b
                    .path_segments
                    .last()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                symptoms.push(
                    Diagnostic::new(
                        "use-nested-package-not-found",
                        format!(
                            "no nested package `{}/{}` (expected a folder module with flask.jsonc)",
                            dep_dir.display(),
                            segment
                        ),
                    )
                    .with_help(
                        "create `segment/flask.jsonc` under the parent package, or fix the import path",
                    )
                    .with_arg("parent", dep_dir.display().to_string())
                    .with_arg("segment", segment.clone())
                    .at_span(spans.get(span_id)),
                );
                return ResolvedModule {
                    files: Vec::new(),
                    symbol_aliases: Vec::new(),
                };
            }
        }
    };

    let qual_prefix = b.qualifier_prefix();
    let mut symbol_aliases = Vec::new();
    let mut files = Vec::new();
    for m in &b.members {
        let (mut member_files, alias) =
            resolve_bundle_member_at(module_loader, &module_dir, &qual_prefix, m, spans, symptoms);
        files.append(&mut member_files);
        if let Some(a) = alias {
            symbol_aliases.push(a);
        }
    }

    ResolvedModule {
        files,
        symbol_aliases,
    }
}

fn resolve_package_like_import(
    module_import: &ModuleImport,
    mp: &Spanned<ModPath>,
    module_loader: &mut ModuleLoader,
    env: ResolveImportEnv<'_>,
) -> ResolvedModule {
    let root_name = mp.root.as_str();
    let Some(dep_dir) = env.dependencies.get(root_name) else {
        env.symptoms.push(
            Diagnostic::new(
                "use-unknown-dependency",
                format!(
                    "unknown dependency `{}` (not found in flask.jsonc dependencies)",
                    root_name
                ),
            )
            .with_help("add it to `dependencies` in flask.jsonc, or use a local file import")
            .with_arg("name", root_name.to_string())
            .at_span(env.spans.get(env.span_id)),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    };

    if !dep_dir.is_folder_module_dir() {
        env.symptoms.push(
            Diagnostic::new(
                "use-dependency-missing-config",
                format!(
                    "dependency `{}` has no flask.jsonc at {}",
                    root_name,
                    dep_dir.display()
                ),
            )
            .with_help("add a flask.jsonc to the dependency root directory")
            .with_arg("name", root_name.to_string())
            .with_arg("path", dep_dir.display().to_string())
            .at_span(env.spans.get(env.span_id)),
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
            files: resolve_folder_module_files(
                module_loader,
                dep_dir,
                &eff_root,
                env.spans,
                env.span_id,
                env.symptoms,
            ),
            symbol_aliases: Vec::new(),
        };
    }

    let seg_strs: Vec<&str> = mp.segments.iter().map(|s| s.as_str()).collect();
    let (module_segs, trailing_member) = split_dep_path_segments(dep_dir, &seg_strs);

    if let Some(member_name) = trailing_member {
        let module_dir = if module_segs.is_empty() {
            dep_dir.clone()
        } else {
            let refs: Vec<&str> = module_segs.iter().map(|s| s.as_str()).collect();
            match resolve_logical_module_dir(dep_dir, &refs) {
                Some(d) => d,
                None => {
                    env.symptoms.push(
                        Diagnostic::new(
                            "use-nested-package-not-found",
                            format!(
                                "no nested package `{}/{}` (expected a folder module with flask.jsonc)",
                                dep_dir.display(),
                                member_name,
                            ),
                        )
                        .with_help(
                            "create `segment/flask.jsonc` under the parent package, or fix the import path",
                        )
                        .with_arg("parent", dep_dir.display().to_string())
                        .with_arg("segment", member_name.clone())
                        .at_span(env.spans.get(env.span_id)),
                    );
                    return ResolvedModule {
                        files: Vec::new(),
                        symbol_aliases: Vec::new(),
                    };
                }
            }
        };

        let member = BundleExportImport {
            export: internment::Intern::new(member_name.clone()),
            alias: module_import.alias,
            span: mp.span_id(),
        };

        let Some(member_file) = module_loader.find_public_def(&module_dir, &member_name) else {
            env.symptoms.push(
                Diagnostic::new(
                    "use-not-exported",
                    format!(
                        "`{}` is not exported from `{}`",
                        member_name,
                        module_dir.display()
                    ),
                )
                .with_help(format!(
                    "`{member_name}` is not exported from `{}`",
                    module_dir.display()
                ))
                .with_arg("symbol", member_name.clone())
                .with_arg("module", module_dir.display().to_string())
                .at_span(env.spans.get(mp.span_id())),
            );
            return ResolvedModule {
                files: Vec::new(),
                symbol_aliases: Vec::new(),
            };
        };

        let mut qual_parts = vec![root_name.to_string()];
        qual_parts.extend(module_segs);
        let qual = qual_parts.join(".");
        return ResolvedModule {
            files: vec![(member_file, qual.clone())],
            symbol_aliases: vec![flat_member_symbol_alias(&qual, &member)],
        };
    }

    let refs: Vec<&str> = module_segs.iter().map(|s| s.as_str()).collect();
    let Some(module_dir) = resolve_logical_module_dir(dep_dir, &refs) else {
        env.symptoms.push(
            Diagnostic::new(
                "use-nested-package-not-found",
                format!(
                    "no nested package `{}/{}` (expected a folder module with flask.jsonc)",
                    dep_dir.display(),
                    module_segs
                        .last()
                        .cloned()
                        .unwrap_or_else(|| "?".to_string()),
                ),
            )
            .with_help(
                "create `segment/flask.jsonc` under the parent package, or fix the import path",
            )
            .with_arg("parent", dep_dir.display().to_string())
            .with_arg(
                "segment",
                module_segs
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "?".to_string()),
            )
            .at_span(env.spans.get(env.span_id)),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    };

    let eff = module_import
        .alias
        .as_ref()
        .map(|a| a.to_string())
        .unwrap_or_else(|| {
            module_segs
                .last()
                .cloned()
                .unwrap_or_else(|| root_name.to_string())
        });
    let qual = if module_import.alias.is_some() {
        eff
    } else {
        let mut parts = vec![root_name.to_string()];
        parts.extend(module_segs.clone());
        parts.join(".")
    };

    ResolvedModule {
        files: resolve_folder_module_files(
            module_loader,
            &module_dir,
            &qual,
            env.spans,
            env.span_id,
            env.symptoms,
        ),
        symbol_aliases: Vec::new(),
    }
}

/// All `.gin` files for a logical folder module (non-recursive), same qualifier.
fn resolve_folder_module_files(
    module_loader: &mut ModuleLoader,
    module_dir: &Path,
    qual_prefix: &str,
    spans: &SpanTable,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    let paths = module_loader.load_module(module_dir);
    if paths.is_empty() {
        symptoms.push(
            Diagnostic::new(
                "use-package-no-gin-files",
                format!(
                    "folder module `{}` contains no `.gin` source files",
                    module_dir.display()
                ),
            )
            .with_help("add at least one `.gin` file next to flask.jsonc")
            .at_span(spans.get(span_id)),
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
    use crate::file_helpers::GinPackageExt;
    use crate::graph::{ResolveGraph, ResolveNode, discovery};
    use crate::module_inventory::ModuleInventory;
    use crate::module_loader::ModuleLoader;
    use diagnostic::Span;
    use internment::Intern;
    use parser::query::SourceParseExt;

    fn make_pf(path: &str, source: &str) -> ParsedFile {
        let output = source.parse_source_full();
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

        let inventory = ModuleInventory::discover(&HashMap::new());
        let mut loader = ModuleLoader::new(inventory, available.into_values());
        let graph = discovery(&mut loader, &[PathBuf::from("main.gin")], &HashMap::new());

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
                ModPath::new(
                    Intern::<String>::from_ref("dep"),
                    vec![Intern::<String>::from_ref("bar")],
                ),
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

        let inventory = ModuleInventory::discover(&HashMap::new());
        let mut loader = ModuleLoader::new(inventory, available.into_values());
        let graph = discovery(&mut loader, &[PathBuf::from("main.gin")], &HashMap::new());

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
        let pf = make_pf("/main.gin", "x: 42\n");
        let mut symptoms = vec![];
        let diag = Diagnostic::new(
            "use-duplicate-top-level",
            "duplicate top-level definition `x` when merging module files",
        )
        .with_help(
            "rename or move one of the definitions so each public top-level name is unique in the package",
        )
        .with_arg("symbol", "x")
        .at_span(Span::new(0, 0));

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

        let symbols = pkg_dir.list_public_symbols();

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
