use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ast::ImportSource;
use ast::{FileAst, LocalBundleImport, MergeConflict, ModuleImport, qualify_module_defs, HasSpanId};
use diagnostic::{Diagnostic, DiagnosticLike, ImportSymptom, SpanId};
use flask::{DependencyKind, FlaskConfig, PACKAGE_CONFIG_NAME};
use internment::Intern;

use parser::parse_source_full;

use crate::ParsedFile;

fn is_folder_module_dir(path: &Path) -> bool {
    path.is_dir() && path.join(PACKAGE_CONFIG_NAME).is_file()
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
                files[from_idx]
                    .output
                    .symptoms
                    .extend(import_symptoms);

                for (file_path, qual) in resolved {
                    if !file_path.is_file() {
                        files[from_idx].output.symptoms.push(
                            ImportSymptom::TargetNotFound {
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
                            ImportSymptom::Conflict {
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
                                eprintln!(
                                    "Error reading import {}: {}",
                                    file_path.display(),
                                    err
                                );
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
            .push(ImportSymptom::Cycle { chain }.into_diagnostic(cycle.closing_span));
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
            ImportSymptom::FolderMissingConfig {
                folder: package_dir.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
        return Vec::new();
    }

    let paths = flask::list_package_gin_files(package_dir);
    if paths.is_empty() {
        symptoms.push(
            ImportSymptom::PackageHasNoGinFiles {
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
) -> Vec<(PathBuf, String)> {
    let root_name = b.root.as_str();
    let Some(dep_dir) = dependencies.get(root_name) else {
        symptoms.push(
            ImportSymptom::UnknownDependency {
                name: root_name.to_string(),
            }
            .into_diagnostic(span_id),
        );
        return Vec::new();
    };

    if !is_folder_module_dir(dep_dir) {
        symptoms.push(
            ImportSymptom::DependencyMissingConfig {
                name: root_name.to_string(),
                path: dep_dir.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
        return Vec::new();
    }

    let mut out = Vec::new();
    for m in &b.members {
        let nested = dep_dir.join(m.export.as_str());
        if !is_folder_module_dir(&nested) {
            symptoms.push(
                ImportSymptom::NestedPackageNotFound {
                    parent: dep_dir.display().to_string(),
                    segment: m.export.to_string(),
                }
                .into_diagnostic(span_id),
            );
            continue;
        }
        let qual = m
            .alias
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_else(|| m.export.to_string());
        out.extend(resolve_package_gin_files(&nested, &qual, span_id, symptoms));
    }
    out
}

fn resolve_local_path_import(
    module_import: &ModuleImport,
    base_dir: &Path,
    path: &Path,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    let full = base_dir.join(path);

    if full.is_dir() && is_folder_module_dir(&full) {
        if module_import.alias.is_none() {
            symptoms.push(
                ImportSymptom::LocalFolderRequiresAs {
                    path: full.display().to_string(),
                }
                .into_diagnostic(span_id),
            );
            return Vec::new();
        }
        let qual = module_import.alias.as_ref().unwrap().to_string();
        return resolve_package_gin_files(&full, &qual, span_id, symptoms);
    }

    let gin_path = if full.is_file() && full.extension().is_some_and(|e| e == "gin") {
        full.clone()
    } else {
        let with_gin = full.with_extension("gin");
        if with_gin.is_file() {
            with_gin
        } else {
            symptoms.push(
                ImportSymptom::LocalNotFound {
                    path: base_dir.join(path).display().to_string(),
                }
                .into_diagnostic(span_id),
            );
            return Vec::new();
        }
    };

    if !gin_path.is_file() {
        symptoms.push(
            ImportSymptom::LocalNotFound {
                path: gin_path.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
        return Vec::new();
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
    vec![(gin_path, qual)]
}

fn resolve_module_import(
    module_import: &ModuleImport,
    base_dir: &Path,
    dependencies: &HashMap<String, PathBuf>,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    match &module_import.source {
        ImportSource::Local(path, _) => resolve_local_path_import(module_import, base_dir, path, span_id, symptoms),
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
) -> Vec<(PathBuf, String)> {
    let root_name = mp.root.as_str();
    let Some(dep_dir) = dependencies.get(root_name) else {
        symptoms.push(
            ImportSymptom::UnknownDependency {
                name: root_name.to_string(),
            }
            .into_diagnostic(span_id),
        );
        return Vec::new();
    };

    if !is_folder_module_dir(dep_dir) {
        symptoms.push(
            ImportSymptom::DependencyMissingConfig {
                name: root_name.to_string(),
                path: dep_dir.display().to_string(),
            }
            .into_diagnostic(span_id),
        );
        return Vec::new();
    }

    if mp.segments.is_empty() {
        let eff_root = module_import
            .alias
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_else(|| root_name.to_string());
        return resolve_package_gin_files(dep_dir, &eff_root, span_id, symptoms);
    }

    let chain = mp
        .segments
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(".");
    let eff = match &module_import.alias {
        Some(a) => format!("{a}.{chain}"),
        None => chain,
    };

    resolve_nested_package_gin_files_from_dir(dep_dir, &eff, &mp.segments, span_id, symptoms)
}

fn resolve_nested_package_gin_files_from_dir(
    start_dir: &Path,
    effective_prefix: &str,
    segments: &[Intern<String>],
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    let segs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    let target = match flask::resolve_nested_package_path(start_dir, &segs) {
        Ok(t) => t,
        Err(err) => {
            match err {
                flask::NestedPackageResolveError::MissingConfig { dir } => symptoms.push(
                    ImportSymptom::MissingConfig {
                        dir: dir.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                ),
                flask::NestedPackageResolveError::NestedPackageNotFound { parent, segment } => {
                    symptoms.push(
                        ImportSymptom::NestedPackageNotFound {
                            parent: parent.display().to_string(),
                            segment,
                        }
                        .into_diagnostic(span_id),
                    );
                }
                flask::NestedPackageResolveError::IntermediateNotFolderModule { path } => {
                    symptoms.push(
                        ImportSymptom::ChainedExportNotFolder {
                            path: path.display().to_string(),
                        }
                        .into_diagnostic(span_id),
                    );
                }
            }
            return Vec::new();
        }
    };

    match target {
        flask::NestedPackageTarget::FolderModule(folder) => {
            if !is_folder_module_dir(&folder) {
                symptoms.push(
                    ImportSymptom::FolderMissingConfig {
                        folder: folder.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            }
            resolve_package_gin_files(&folder, effective_prefix, span_id, symptoms)
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
            errors.push(ImportSymptom::DuplicateTopLevel { symbol }.into_diagnostic(span));
        }
    }
    if errors.is_empty() {
        Ok(merged)
    } else {
        Err(errors)
    }
}
