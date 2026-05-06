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
    if let Some(word) = typeck::word_at_byte_offset(source, byte_pos) {
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

/// Resolve imports for a parsed package.
///
/// When `deps` is `Some`, this expands all imports discovered transitively from
/// the entry file: parsing imported files, qualifying their definitions, and
/// detecting import cycles. When `deps` is `None` (library mode), files are
/// passed through unchanged.
pub fn resolve_imports(
    files: Vec<ParsedFile>,
    deps: Option<&HashMap<String, PathBuf>>,
) -> Vec<ParsedFile> {
    let Some(deps) = deps else {
        return files;
    };

    // Nothing to resolve if there are no files.
    if files.is_empty() {
        return files;
    }

    let mut files = files;

    let entry_path = files[0].path.clone();
    let _entry_dir = entry_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    let mut seen: HashMap<PathBuf, String> = HashMap::new();
    let mut node_by_path: HashMap<PathBuf, usize> = HashMap::new();
    node_by_path.insert(entry_path.clone(), 0);
    seen.insert(entry_path.clone(), String::new());

    let mut adj: Vec<Vec<crate::module_graph::ImportEdge>> = vec![Vec::new()];
    let mut processed_imports: Vec<bool> = vec![false];

    loop {
        let next = processed_imports
            .iter()
            .enumerate()
            .find_map(|(i, done)| (!done).then_some(i));
        let Some(from_idx) = next else { break };
        processed_imports[from_idx] = true;

        let from_path = files[from_idx].path.clone();
        let from_dir = from_path.parent().unwrap_or(Path::new("")).to_path_buf();
        let from_ast = files[from_idx].output.ast.clone();

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
                );
                files[from_idx].output.symptoms.extend(import_symptoms);

                for alias in resolved.symbol_aliases {
                    files[from_idx].output.ast.symbol_aliases.push(alias);
                }

                for (file_path, qual) in resolved.files {
                    if !file_path.is_file() {
                        files[from_idx].output.symptoms.push(
                            UseSymptom::TargetNotFound {
                                path: file_path.display().to_string(),
                            }
                            .into_diagnostic(span_id),
                        );
                        continue;
                    }

                    if let Some(prev) = seen.get(&file_path)
                        && prev != &qual
                    {
                        files[from_idx].output.symptoms.push(
                            UseSymptom::Conflict {
                                path: file_path.display().to_string(),
                                qualifier_a: prev.clone(),
                                qualifier_b: qual,
                            }
                            .into_diagnostic(span_id),
                        );
                        continue;
                    }

                    let to_idx = if let Some(i) = node_by_path.get(&file_path).copied() {
                        i
                    } else {
                        let source = match std::fs::read_to_string(&file_path) {
                            Ok(s) => s,
                            Err(err) => {
                                eprintln!("Error reading import {}: {}", file_path.display(), err);
                                continue;
                            }
                        };
                        let mut output = parse_source_full(&source);
                        output.ast = qualify_module_defs(output.ast, &qual);
                        output.ast = output.ast.strip_private_for_importer();

                        let i = files.len();
                        files.push(ParsedFile {
                            path: file_path.clone(),
                            source,
                            output,
                        });
                        node_by_path.insert(file_path.clone(), i);
                        adj.push(Vec::new());
                        processed_imports.push(false);
                        seen.insert(file_path.clone(), qual.clone());
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
            parts.push(files[n].path.display().to_string());
        }
        let chain = parts.join(" -> ");

        files[cycle.closing_from]
            .output
            .symptoms
            .push(UseSymptom::Cycle { chain }.into_diagnostic(cycle.closing_span));
    }

    for file in &mut files {
        apply_symbol_aliases(&mut file.output.ast);
        file.output.ast.symbol_aliases.clear();
    }

    files
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
            let found = check_public_def_in_package(dep_dir, m.export.as_str());
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
) -> ResolvedModule {
    match &module_import.source {
        ImportSource::Local(path, _) => {
            resolve_local_path_import(module_import, base_dir, path, span_id, symptoms)
        }
        ImportSource::LocalBundle(b) => {
            resolve_dependency_bundle_import(b, dependencies, span_id, symptoms)
        }
        ImportSource::Package(mp) => resolve_package_like_import(
            module_import,
            mp,
            base_dir,
            dependencies,
            span_id,
            symptoms,
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
                if let Some(file_path) = find_public_def_in_package(dep_dir, symbol.as_str()) {
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
    let span = SpanId::INVALID;
    for file in files {
        if let Err(conflict) = merged.merge_from_checked(file.output.ast.clone()) {
            let symbol = match conflict {
                MergeConflict::Tag { name } | MergeConflict::Def { name } => name.to_string(),
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
