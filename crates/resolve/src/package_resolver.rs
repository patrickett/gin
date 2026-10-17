use diagnostic::code::ImportCode;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use ast::{
    BundleExportImport, ImportSource, LocalBundleImport, ModPath, ModuleImport, Spanned,
    SymbolAlias,
};
use diagnostic::{Diagnostic, DiagnosticPathExt, SpanId, SpanTable};

use crate::ParsedFile;

use crate::folder_module::{
    NormalizePathError, normalize_local_import_path, resolve_logical_module_dir,
    split_dep_path_segments,
};
use crate::graph::{ResolveGraph, build_import_closure_with_cache};
use crate::module_loader::{ModuleLoader, ParsedModuleCache};
use flask::FlaskPathExt;

#[derive(Debug, Clone, Default)]
pub struct ImportDependencyGraph {
    /// File path → paths it imports (sorted, deduplicated).
    imports_by_importer: HashMap<PathBuf, Vec<PathBuf>>,
    scope_roots: Vec<PathBuf>,
}

impl ImportDependencyGraph {
    fn from_resolve_graph(graph: &ResolveGraph) -> Self {
        let mut imports_by_importer: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

        for node in &graph.nodes {
            imports_by_importer.entry(node.path.clone()).or_default();
        }

        for (from_idx, targets) in graph.adj.iter().enumerate() {
            let Some(from_node) = graph.nodes.get(from_idx) else {
                continue;
            };

            for edge in targets {
                let Some(to_node) = graph.nodes.get(edge.to) else {
                    continue;
                };
                imports_by_importer
                    .entry(from_node.path.clone())
                    .or_default()
                    .push(to_node.path.clone());
            }
        }

        for imports in imports_by_importer.values_mut() {
            imports.sort();
            imports.dedup();
        }

        let mut scope_roots = Vec::new();
        for package in &graph.packages.nodes {
            scope_roots.push(package.root.normalize_diagnostic_path());
            for dependency in package.config.dependencies().values() {
                if let flask::DependencyKind::Path { path } = &dependency.kind {
                    scope_roots.push(package.root.join(path).normalize_diagnostic_path());
                }
            }
        }
        scope_roots.sort();
        scope_roots.dedup();
        Self {
            imports_by_importer,
            scope_roots,
        }
    }

    pub fn impacted_modules(&self, path: &Path) -> Vec<PathBuf> {
        if !self.imports_by_importer.contains_key(path) {
            if self.scope_roots.iter().any(|root| path.starts_with(root)) {
                let mut files: Vec<_> = self.imports_by_importer.keys().cloned().collect();
                files.sort();
                return files;
            }
            return Vec::new();
        }

        let mut importers_by_imported: HashMap<&Path, Vec<&Path>> = HashMap::new();
        for (importer, imports) in &self.imports_by_importer {
            for imported in imports {
                importers_by_imported
                    .entry(imported.as_path())
                    .or_default()
                    .push(importer.as_path());
            }
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
            for importer in importers_by_imported
                .get(current.as_path())
                .into_iter()
                .flatten()
            {
                queue.push_back(importer.to_path_buf());
            }
        }

        impacted
    }

    /// Return file paths ordered so imported files are declared before importers.
    pub fn declaration_file_order(&self, file_paths: &[PathBuf]) -> Vec<PathBuf> {
        fn visit_imports(
            path: &PathBuf,
            imports_by_importer: &HashMap<PathBuf, Vec<PathBuf>>,
            visited: &mut HashSet<PathBuf>,
            sorted: &mut Vec<PathBuf>,
        ) {
            if visited.contains(path) {
                return;
            }

            visited.insert(path.clone());

            if let Some(deps) = imports_by_importer.get(path) {
                for dep in deps {
                    visit_imports(dep, imports_by_importer, visited, sorted);
                }
            }

            sorted.push(path.clone());
        }

        let mut visited = HashSet::new();
        let mut ordered = Vec::with_capacity(file_paths.len());
        let mut file_paths = file_paths.to_vec();
        file_paths.sort();
        file_paths.dedup();
        for path in &file_paths {
            visit_imports(path, &self.imports_by_importer, &mut visited, &mut ordered);
        }

        ordered
    }
}

pub(crate) struct ResolveImportsWithGraph {
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
    } = resolve_imports_with_home(
        entry_files,
        dependencies,
        ParsedModuleCache::default(),
        None,
    );
    files
}

pub(crate) fn resolve_imports_with_home(
    entry_files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
    cache: ParsedModuleCache,
    home: Option<&flask::GinHome>,
) -> ResolveImportsWithGraph {
    let (graph, cache) = build_import_closure_with_cache(entry_files, dependencies, cache, home);
    let import_graph = ImportDependencyGraph::from_resolve_graph(&graph);
    let files = resolve(graph, &mut |path| cache.files.get(path).cloned());
    ResolveImportsWithGraph {
        files,
        cache,
        import_graph,
    }
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
        packages,
        mut staged_asts,
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

        let package = packages.node(node.package);
        let staged = staged_asts.remove(&node.path);
        let qualified = staged.is_some();
        if let Some(ast) = staged {
            parsed.output.ast = ast;
        }
        parsed.output.ast.semantic_origin = Some(ast::prelude::FileSemanticOrigin {
            package: package.instance.clone(),
            module: packages.module_path(node.package, &node.path),
        });

        if !node.qualifier.is_empty() {
            if !qualified {
                parsed.output.ast = parsed.output.ast.qualify_module_defs(&node.qualifier);
            }
            if !node.is_entry {
                parsed.output.ast = parsed.output.ast.strip_private_for_importer();
            }
        }

        parsed.output.ast.symbol_aliases = node_aliases[i].clone();
        parsed.output.ast.apply_symbol_aliases();
        parsed.output.ast.symbol_aliases.clear();

        files.push(Some(parsed));
    }

    for (node_idx, diag) in symptoms {
        if let Some(ref mut f) = files[node_idx] {
            f.output.symptoms.push(diag);
        }
    }

    files.into_iter().flatten().collect()
}

#[derive(Default)]
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
        ImportSource::LocalFile(path, _) => resolve_local_file_import(
            path,
            module_import.alias.as_ref().map(|a| a.as_ref().as_str()),
            base_dir,
            module_loader,
            spans,
            span_id,
            symptoms,
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
    }
}

fn resolve_local_file_import(
    path: &Path,
    alias: Option<&str>,
    base_dir: &Path,
    loader: &mut ModuleLoader,
    spans: &SpanTable,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> ResolvedModule {
    let Some(alias) = alias else {
        return ResolvedModule::default();
    };
    let Some(pkg_root) = crate::import_query::find_package_root(base_dir) else {
        return ResolvedModule::default();
    };
    let candidate = base_dir.join(path);
    let full = match candidate.canonicalize() {
        Ok(full)
            if full.starts_with(&pkg_root)
                && full.is_file()
                && full.extension().is_some_and(|e| e == "gin") =>
        {
            full
        }
        Ok(full) => {
            symptoms.push(
                // use-local-file-not-found
                Diagnostic::new(
                    ImportCode::LocalFileNotFound,
                    format!("local file import not found: `{}`", full.display()),
                )
                .at_span(spans.get(span_id)),
            );
            return ResolvedModule::default();
        }
        Err(_) => {
            symptoms.push(
                // use-local-file-not-found
                Diagnostic::new(
                    ImportCode::LocalFileNotFound,
                    format!("local file import not found: `{}`", path.display()),
                )
                .with_help("check the path relative to the importing file")
                .at_span(spans.get(span_id)),
            );
            return ResolvedModule::default();
        }
    };
    if loader.parsed_file(&full).is_none() {
        loader.load_containing_module(&full);
    }
    if loader.parsed_file(&full).is_none() {
        symptoms.push(
            // use-local-file-not-found
            Diagnostic::new(
                ImportCode::LocalFileNotFound,
                format!("local file import not found: `{}`", full.display()),
            )
            .at_span(spans.get(span_id)),
        );
        return ResolvedModule::default();
    }
    ResolvedModule {
        files: vec![(full, alias.to_string())],
        symbol_aliases: vec![],
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
            not_exported_diagnostic(symbol, &file_dir.display().to_string())
                .at_span(spans.get(member.span)),
        );
        return ResolvedModule::default();
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

/// Resolve one `use dep folder.Symbol, …` entry against `module_dir`.
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
                            not_exported_diagnostic(export, qual_prefix)
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

    symptoms.push(not_exported_diagnostic(export, qual_prefix).at_span(spans.get(member.span)));
    (Vec::new(), None)
}

fn local_module_qualifier(base_dir: &Path, module_dir: &Path) -> Option<String> {
    let (config, root) = flask::FlaskConfig::find_package_config(base_dir)?;
    let module = module_dir.strip_prefix(root).ok()?;
    Some(
        std::iter::once(config.name())
            .chain(
                module
                    .components()
                    .filter_map(|component| component.as_os_str().to_str()),
            )
            .collect::<Vec<_>>()
            .join("."),
    )
}

fn not_exported_diagnostic(export: &str, module: &str) -> Diagnostic {
    // use-not-exported
    Diagnostic::new(
        ImportCode::NotExported,
        format!("`{export}` is not exported from `{module}`"),
    )
    .with_help(format!("`{export}` is not exported from `{module}`"))
    .with_arg("symbol", export.to_string())
    .with_arg("module", module.to_string())
}

fn report_normalize_path_error(
    error: NormalizePathError,
    spans: &SpanTable,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) {
    let diagnostic = match error {
        // use-import-target-must-be-folder
        NormalizePathError::ImportTargetMustBeFolder { path } => Diagnostic::new(
            ImportCode::ImportTargetMustBeFolder,
            format!(
                "`{}` must be a folder to import as a folder module",
                path.display()
            ),
        )
        .with_help("use `use './file.gin'` for local file imports"),
        // use-escapes-package-root
        NormalizePathError::EscapesPackageRoot { path, .. } => Diagnostic::new(
            ImportCode::EscapesPackageRoot,
            format!("import path `{}` escapes the package root", path.display()),
        )
        .with_help("ensure the import path stays within the current package"),
        // use-local-not-found
        NormalizePathError::NotADirectory(path) => Diagnostic::new(
            ImportCode::LocalNotFound,
            format!("local import not found: `{}`", path.display()),
        )
        .with_help("check the path relative to this file, and ensure it ends in `.gin`"),
    };
    symptoms.push(diagnostic.at_span(spans.get(span_id)));
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
        Err(error) => {
            report_normalize_path_error(error, spans, span_id, symptoms);
            None
        }
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
            // use-local-folder-requires-as
            Diagnostic::new(
                ImportCode::LocalFolderRequiresAs,
                format!(
                    "folder module `{}` must be imported with `as` (e.g. `use '{}' as name`)",
                    path.display(),
                    path.display()
                ),
            )
            .with_help("add `as Alias` so the folder module has a single namespace prefix")
            .at_span(spans.get(span_id)),
        );
        return ResolvedModule::default();
    };

    let Some(full) = resolve_local_folder_module(base_dir, path, spans, span_id, symptoms) else {
        return ResolvedModule::default();
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

    let Some(qual) = local_module_qualifier(base_dir, &full) else {
        return ResolvedModule::default();
    };
    let mut parts = qual.split('.');
    let target = ModPath::new(
        internment::Intern::new(parts.next().unwrap().to_owned()),
        parts
            .map(|part| internment::Intern::new(part.to_owned()))
            .collect(),
    );
    let symbol_alias = SymbolAlias {
        alias: internment::Intern::new(alias.to_owned()),
        target: Spanned::new(target, span_id),
    };
    let gin_files =
        resolve_folder_module_files(module_loader, &full, &qual, spans, span_id, symptoms);
    if gin_files.is_empty() {
        symptoms.push(
            // use-package-no-gin-files
            Diagnostic::new(
                ImportCode::PackageNoGinFiles,
                format!(
                    "folder module `{}` contains no `.gin` source files",
                    full.display()
                ),
            )
            .with_help("add at least one `.gin` file next to mod.flask")
            .at_span(spans.get(span_id)),
        );
    }
    ResolvedModule {
        files: gin_files,
        symbol_aliases: vec![symbol_alias],
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
            // use-dep-local-name-collision
            Diagnostic::new(
                ImportCode::DepLocalNameCollision,
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

/// Resolve a local-path bundle import like `use 'arch' Constraint, Register, X0`.
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
            return ResolvedModule::default();
        }
    };

    let Some(module_dir) =
        resolve_local_folder_module(base_dir, local_path, spans, span_id, symptoms)
    else {
        return ResolvedModule::default();
    };

    let Some(qual) = local_module_qualifier(base_dir, &module_dir) else {
        return ResolvedModule::default();
    };
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
            // use-unknown-dependency
            Diagnostic::new(
                ImportCode::UnknownDependency,
                format!(
                    "unknown dependency `{}` (not found in mod.flask dependencies)",
                    root_name
                ),
            )
            .with_help("add it to `dependencies` in mod.flask, or use a local file import")
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
            // use-dependency-missing-config
            Diagnostic::new(
                ImportCode::DependencyMissingConfig,
                format!(
                    "dependency `{}` has no mod.flask at {}",
                    root_name,
                    dep_dir.display()
                ),
            )
            .with_help("add a mod.flask to the dependency root directory")
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
                    // use-nested-package-not-found
                    Diagnostic::new(
                        ImportCode::NestedPackageNotFound,
                        format!(
                            "no nested package `{}/{}` (expected a folder module with mod.flask)",
                            dep_dir.display(),
                            segment
                        ),
                    )
                    .with_help(
                        "create `segment/mod.flask` under the parent package, or fix the import path",
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
            // use-unknown-dependency
            Diagnostic::new(
                ImportCode::UnknownDependency,
                format!(
                    "unknown dependency `{}` (not found in mod.flask dependencies)",
                    root_name
                ),
            )
            .with_help("add it to `dependencies` in mod.flask, or use a local file import")
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
            // use-dependency-missing-config
            Diagnostic::new(
                ImportCode::DependencyMissingConfig,
                format!(
                    "dependency `{}` has no mod.flask at {}",
                    root_name,
                    dep_dir.display()
                ),
            )
            .with_help("add a mod.flask to the dependency root directory")
            .with_arg("name", root_name.to_string())
            .with_arg("path", dep_dir.display().to_string())
            .at_span(env.spans.get(env.span_id)),
        );
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    }

    let segments: Vec<&str> = mp.segments.iter().map(|s| s.as_str()).collect();
    let Some(module_dir) = resolve_logical_module_dir(dep_dir, &segments) else {
        let path = std::iter::once(root_name)
            .chain(segments.iter().copied())
            .collect::<Vec<_>>()
            .join(".");
        // use-module-not-found
        let mut diagnostic = Diagnostic::new(
            ImportCode::ModuleNotFound,
            format!("`{path}` does not name a module"),
        )
        .with_arg("module", &path);
        if let Some((last, parent)) = segments.split_last() {
            let parent = std::iter::once(root_name)
                .chain(parent.iter().copied())
                .collect::<Vec<_>>()
                .join(".");
            diagnostic =
                diagnostic.with_help(format!("to select an export, write `use {parent} {last}`"));
        }
        env.symptoms
            .push(diagnostic.at_span(env.spans.get(mp.span_id())));
        return ResolvedModule {
            files: Vec::new(),
            symbol_aliases: Vec::new(),
        };
    };
    let qualifier = std::iter::once(root_name)
        .chain(segments.iter().copied())
        .collect::<Vec<_>>()
        .join(".");
    let alias = module_import
        .alias
        .unwrap_or_else(|| mp.segments.last().copied().unwrap_or(mp.root));
    let symbol_aliases = vec![SymbolAlias {
        alias,
        target: Spanned::new(mp.value.clone(), mp.span_id()),
    }];
    ResolvedModule {
        files: resolve_folder_module_files(
            module_loader,
            &module_dir,
            &qualifier,
            env.spans,
            env.span_id,
            env.symptoms,
        ),
        symbol_aliases,
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
            // use-package-no-gin-files
            Diagnostic::new(
                ImportCode::PackageNoGinFiles,
                format!(
                    "folder module `{}` contains no `.gin` source files",
                    module_dir.display()
                ),
            )
            .with_help("add at least one `.gin` file next to mod.flask")
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

    #[test]
    fn declaration_order_is_stable_across_root_and_edge_insertion_orders() {
        let edges = [
            ("a.gin", vec!["b.gin", "c.gin"]),
            ("b.gin", vec!["d.gin"]),
            ("c.gin", vec!["d.gin"]),
        ];
        for reverse in [false, true] {
            let mut graph = ImportDependencyGraph::default();
            let mut edges = edges.to_vec();
            if reverse {
                edges.reverse();
            }
            for (importer, dependencies) in edges {
                graph.imports_by_importer.insert(
                    importer.into(),
                    dependencies.into_iter().map(PathBuf::from).collect(),
                );
            }
            for roots in [
                vec!["a.gin", "c.gin", "b.gin", "d.gin"],
                vec!["c.gin", "b.gin", "d.gin", "a.gin"],
            ] {
                assert_eq!(
                    graph.declaration_file_order(
                        &roots.into_iter().map(PathBuf::from).collect::<Vec<_>>()
                    ),
                    ["d.gin", "b.gin", "c.gin", "a.gin"].map(PathBuf::from)
                );
            }
        }
    }

    #[test]
    fn source_first_imports_resolve_folder_exports_and_aliases() {
        let dir = std::env::temp_dir().join(format!("gin-source-import-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("math")).unwrap();
        std::fs::write(
            dir.join("mod.flask"),
            r#"#settings
module: "sample"
version: 0.0.0

#dependencies
"#,
        )
        .unwrap();
        std::fs::write(dir.join("math/ops.gin"), "add := 1\nsub := 2\n").unwrap();
        let dir = dir.canonicalize().unwrap();
        let path = dir.join("main.gin");
        for import in [
            "use './math' add, sub as subtract",
            "use './math'\n    add\n    sub as subtract",
        ] {
            std::fs::write(&path, format!("{import}\n\nmain: subtract\n")).unwrap();
            let files = resolve_imports(vec![ParsedFile::read(&path).unwrap()], &HashMap::new());
            for file in &files {
                assert!(
                    file.output.symptoms.is_empty(),
                    "{:?}",
                    file.output.symptoms
                );
            }
            assert_eq!(files.len(), 2);
            let entry = files.iter().find(|file| file.path == path).unwrap();
            let main = entry
                .output
                .ast
                .defs
                .values()
                .find(|bind| bind.name.as_str() == "main")
                .unwrap();
            let ast::BindValue::Expr(expr) = &main.value else {
                panic!("expected main expression");
            };
            let ast::Expr::FnCall(call) = &expr.value else {
                panic!("expected imported reference");
            };
            assert_eq!(call.path.segments.last().unwrap().as_str(), "sub");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod namespace_tests {
    use super::*;
    use test_fixtures::{unique_temp_dir, write_file};

    #[test]
    fn imported_entry_files_have_the_same_names_as_imported_dependencies() {
        let dir = unique_temp_dir("imported_entries");
        write_file(
            &dir.join("mod.flask"),
            r#"#settings
module: "sample"
version: 0.0.0

#dependencies
"#,
        );
        write_file(&dir.join("math/ops.gin"), "answer: 42\nprivate hidden: 7\n");
        write_file(
            &dir.join("main.gin"),
            "use sample.math answer\nmain: answer\n",
        );
        let dir = dir.canonicalize().unwrap();
        for paths in [
            vec!["main.gin"],
            vec!["main.gin", "math/ops.gin"],
            vec!["math/ops.gin", "main.gin"],
        ] {
            let entries = paths
                .iter()
                .map(|path| ParsedFile::read(&dir.join(path)).unwrap())
                .collect();
            let files = resolve_imports(entries, &HashMap::new());
            assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
            let math = files
                .iter()
                .find(|file| file.path == dir.join("math/ops.gin"))
                .unwrap();
            assert!(
                math.output
                    .ast
                    .defs
                    .keys()
                    .any(|name| name.0.as_str() == "sample.math.answer")
            );
            assert_eq!(
                math.output
                    .ast
                    .defs
                    .keys()
                    .any(|name| name.0.as_str() == "sample.math.hidden"),
                paths.contains(&"math/ops.gin"),
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relative_entries_keep_module_identity_and_share_imported_files() {
        let dir = unique_temp_dir("relative_entries");
        write_file(
            &dir.join("mod.flask"),
            r#"#settings
module: "sample"
version: 0.0.0

#dependencies
"#,
        );
        write_file(&dir.join("math/ops.gin"), "answer: 42\nprivate hidden: 7\n");
        write_file(
            &dir.join("main.gin"),
            "use sample.math answer\nmain: answer\n",
        );
        let dir = dir.canonicalize().unwrap();
        let cwd = std::env::current_dir().unwrap();
        let common = cwd
            .ancestors()
            .find(|ancestor| dir.starts_with(ancestor))
            .unwrap();
        let relative_dir = cwd
            .strip_prefix(common)
            .unwrap()
            .components()
            .fold(PathBuf::new(), |path, _| path.join(".."))
            .join(dir.strip_prefix(common).unwrap());
        assert!(relative_dir.is_relative());
        let entries = ["main.gin", "math/ops.gin"]
            .into_iter()
            .map(|path| ParsedFile::read(&relative_dir.join(path)).unwrap())
            .collect();
        let files = resolve_imports(entries, &HashMap::new());
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
        let math = files
            .iter()
            .find(|file| file.path == dir.join("math/ops.gin"))
            .unwrap();
        assert_eq!(
            math.output.ast.semantic_origin.as_ref().unwrap().module,
            vec![internment::Intern::new("math".to_owned())]
        );
        assert!(
            math.output
                .ast
                .defs
                .keys()
                .any(|name| name.0.as_str() == "sample.math.hidden")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relative_and_package_imports_share_a_module_with_local_aliases() {
        let dir = unique_temp_dir("relative_package_imports");
        write_file(
            &dir.join("mod.flask"),
            r#"#settings
module: "sample"
version: 0.0.0

#dependencies
"#,
        );
        write_file(&dir.join("math/ops.gin"), "answer: 42\n");
        let cases = [
            (
                "local.gin",
                "use './math' answer as local_answer\nlocal: local_answer\n",
                "local",
            ),
            (
                "nested/main.gin",
                "use '../math' answer\nnested: answer\n",
                "nested",
            ),
            (
                "package.gin",
                "use sample.math answer\npackage: answer\n",
                "package",
            ),
            (
                "alias.gin",
                "use './math' as numbers\naliased: numbers.answer\n",
                "aliased",
            ),
        ];
        let dir = dir.canonicalize().unwrap();
        for reverse in [false, true] {
            let mut entries = cases
                .iter()
                .map(|(name, source, _)| {
                    let path = dir.join(name);
                    write_file(&path, source);
                    ParsedFile::read(&path).unwrap()
                })
                .collect::<Vec<_>>();
            if reverse {
                entries.reverse();
            }
            let files = resolve_imports(entries, &HashMap::new());
            assert_eq!(files.len(), cases.len() + 1);
            for file in &files {
                assert!(
                    file.output.symptoms.is_empty(),
                    "{:?}",
                    file.output.symptoms
                );
            }
            for (name, _, binding) in cases {
                let file = files
                    .iter()
                    .find(|file| file.path == dir.join(name))
                    .unwrap();
                let bind = file
                    .output
                    .ast
                    .defs
                    .values()
                    .find(|bind| bind.name.as_str() == binding)
                    .unwrap();
                let ast::BindValue::Expr(expr) = &bind.value else {
                    panic!("expected expression");
                };
                let ast::Expr::FnCall(call) = &expr.value else {
                    panic!("expected imported reference: {expr:?}");
                };
                assert_eq!(call.path.to_qualified_string(), "sample.math.answer");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn module_names_and_aliases_are_local_to_each_importing_file() {
        let dir = unique_temp_dir("module_namespaces");
        write_file(
            &dir.join("mod.flask"),
            r#"#settings
module: "app"
version: 0.0.0

#dependencies
package: "./dep"
"#,
        );
        write_file(
            &dir.join("dep/mod.flask"),
            r#"#settings
module: "library"
version: 0.0.0

#dependencies
"#,
        );
        write_file(&dir.join("dep/math/ops.gin"), "add: 42\n");
        let dir = dir.canonicalize().unwrap();
        let cases = [
            (
                "main.gin",
                "use package.math\nmain: math.add\n",
                "main",
                "package",
            ),
            (
                "alias.gin",
                "use package.math as numbers\naliased: numbers.add\n",
                "aliased",
                "package",
            ),
            (
                "selected.gin",
                "use package.math add\nselected: add\n",
                "selected",
                "package",
            ),
            ("unbound.gin", "unbound: math.add\n", "unbound", "math"),
            (
                "shadowed.gin",
                "use package.math\nshadowed(math Record): math.add\n",
                "shadowed",
                "math",
            ),
        ];
        let entries = cases
            .iter()
            .map(|(name, source, _, _)| {
                let path = dir.join(name);
                write_file(&path, source);
                ParsedFile::read(&path).unwrap()
            })
            .collect();
        let files = resolve_imports(entries, &HashMap::new());
        for (name, _, binding, root) in cases {
            let file = files
                .iter()
                .find(|file| file.path == dir.join(name))
                .unwrap();
            assert!(
                file.output.symptoms.is_empty(),
                "{:?}",
                file.output.symptoms
            );
            let bind = file
                .output
                .ast
                .defs
                .values()
                .find(|b| b.name.as_str() == binding)
                .unwrap();
            let ast::BindValue::Expr(expr) = &bind.value else {
                panic!("expected expression");
            };
            let value = if root == "math" {
                let ast::Expr::RecordGet { base, field } = &expr.value else {
                    panic!("expected untouched access");
                };
                assert_eq!(field.as_str(), "add");
                &base.value
            } else {
                &expr.value
            };
            let ast::Expr::FnCall(call) = value else {
                panic!("expected imported call in {name}: {expr:?}");
            };
            assert_eq!(call.path.root.as_str(), root);
            assert_eq!(call.path.segments.len(), if root == "math" { 0 } else { 2 });
            if root != "math" {
                assert_eq!(call.path.segments.last().unwrap().as_str(), "add");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dotted_export_import_reports_the_explicit_selection_syntax() {
        let dir = unique_temp_dir("obsolete_export_import");
        write_file(
            &dir.join("mod.flask"),
            r#"#settings
module: "app"
version: 0.0.0

#dependencies
package: "./dep"
"#,
        );
        write_file(
            &dir.join("dep/mod.flask"),
            r#"#settings
module: "library"
version: 0.0.0

#dependencies
"#,
        );
        write_file(&dir.join("dep/math/ops.gin"), "add: 42\n");
        write_file(&dir.join("main.gin"), "use package.math.add\nmain: 0\n");
        let path = dir.join("main.gin").canonicalize().unwrap();
        let files = resolve_imports(vec![ParsedFile::read(&path).unwrap()], &HashMap::new());
        let diagnostic = files
            .iter()
            .flat_map(|file| &file.output.symptoms)
            .find(|d| d.code.slug() == "use-module-not-found")
            .expect("old dotted selection must fail");
        assert!(diagnostic.message.contains("package.math.add"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
