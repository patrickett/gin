use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ast::ImportSource;
use ast::{ModuleImport, qualify_module_defs, HasSpanId};
use diagnostic::{Diagnostic, DiagnosticLike, ImportSymptom, SpanId};
use flask::{DependencyKind, FlaskConfig};
use internment::Intern;

use parser::parse_source_full;

use crate::ParsedFile;

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

enum LocalModuleRoot {
    None,
    Ambiguous,
    File(PathBuf),
    Folder(PathBuf),
}

fn local_module_root(base_dir: &Path, name: &str) -> LocalModuleRoot {
    let file_path = base_dir.join(format!("{name}.gin"));
    let folder = base_dir.join(name);
    let has_file = file_path.is_file();
    let has_folder = folder.is_dir() && folder.join("flask.jsonc").is_file();
    match (has_file, has_folder) {
        (true, true) => LocalModuleRoot::Ambiguous,
        (true, false) => LocalModuleRoot::File(file_path),
        (false, true) => LocalModuleRoot::Folder(folder),
        _ => LocalModuleRoot::None,
    }
}

fn resolve_module_import(
    module_import: &ModuleImport,
    base_dir: &Path,
    dependencies: &HashMap<String, PathBuf>,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    match &module_import.source {
        ImportSource::Local(path, _) => {
            if path.extension().is_none_or(|e| e != "gin") {
                symptoms.push(
                    ImportSymptom::LocalMustEndInGin {
                        path: path.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            }
            let full = base_dir.join(path);
            if !full.is_file() {
                symptoms.push(
                    ImportSymptom::LocalNotFound {
                        path: full.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            }
            let stem = full
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let qual = module_import
                .alias
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or(stem);
            vec![(full, qual)]
        }
        ImportSource::LocalBundle(b) => {
            let folder = base_dir.join(b.root.as_str());
            let Some(config) = FlaskConfig::from_directory(&folder) else {
                symptoms.push(
                    ImportSymptom::FolderMissingConfig {
                        folder: folder.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            };
            let mut out = Vec::new();
            for m in &b.members {
                let Some(spec) = config.exports().get(m.export.as_str()) else {
                    symptoms.push(
                        ImportSymptom::MissingExport {
                            folder: folder.display().to_string(),
                            export: m.export.to_string(),
                        }
                        .into_diagnostic(span_id),
                    );
                    continue;
                };
                let p = folder.join(&spec.path);
                if !p.exists() {
                    symptoms.push(
                        ImportSymptom::ExportTargetNotFound {
                            export: m.export.to_string(),
                            folder: folder.display().to_string(),
                            path: p.display().to_string(),
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
                out.push((p, qual));
            }
            out
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
    base_dir: &Path,
    dependencies: &HashMap<String, PathBuf>,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    let root_name = mp.root.as_str();
    match local_module_root(base_dir, root_name) {
        LocalModuleRoot::Ambiguous => {
            symptoms.push(
                ImportSymptom::AmbiguousLocalRoot {
                    name: root_name.to_string(),
                    file_path: base_dir
                        .join(format!("{root_name}.gin"))
                        .display()
                        .to_string(),
                    folder_path: base_dir.join(root_name).display().to_string(),
                }
                .into_diagnostic(span_id),
            );
            Vec::new()
        }
        LocalModuleRoot::File(f) => {
            if !mp.segments.is_empty() {
                symptoms.push(
                    ImportSymptom::FileHasSegments {
                        file_path: f.display().to_string(),
                        segment: mp.segments[0].to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            }
            let qual = module_import
                .alias
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| root_name.to_string());
            vec![(f, qual)]
        }
        LocalModuleRoot::Folder(folder) => {
            let Some(config) = FlaskConfig::from_directory(&folder) else {
                symptoms.push(
                    ImportSymptom::FolderMissingConfig {
                        folder: folder.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            };
            let eff_root = module_import.alias.as_ref().unwrap_or(&mp.root).to_string();

            if mp.segments.is_empty() {
                return resolve_all_exports(&config, &folder, &eff_root, span_id, symptoms);
            }

            let chain = mp
                .segments
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(".");
            let eff = format!("{eff_root}.{chain}");
            resolve_chained_exports_from_dir(
                module_import,
                &folder,
                &eff,
                &mp.segments,
                span_id,
                symptoms,
            )
        }
        LocalModuleRoot::None => {
            let Some(dep_dir) = dependencies.get(root_name) else {
                symptoms.push(
                    ImportSymptom::UnknownDependency {
                        name: root_name.to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            };
            let Some(config) = FlaskConfig::from_directory(dep_dir) else {
                symptoms.push(
                    ImportSymptom::DependencyMissingConfig {
                        name: root_name.to_string(),
                        path: dep_dir.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            };

            if mp.segments.is_empty() {
                let eff_root = module_import
                    .alias
                    .as_ref()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| root_name.to_string());
                return resolve_all_exports(&config, dep_dir, &eff_root, span_id, symptoms);
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

            resolve_chained_exports_from_dir(
                module_import,
                dep_dir,
                &eff,
                &mp.segments,
                span_id,
                symptoms,
            )
        }
    }
}

fn resolve_chained_exports_from_dir(
    _module_import: &ModuleImport,
    start_dir: &Path,
    effective_prefix: &str,
    segments: &[Intern<String>],
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    let segs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    let target = match flask::resolve_chained_exports(start_dir, &segs) {
        Ok(t) => t,
        Err(err) => {
            match err {
                flask::ExportResolveError::MissingConfig { dir } => symptoms.push(
                    ImportSymptom::MissingConfig {
                        dir: dir.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                ),
                flask::ExportResolveError::MissingExport { dir, key } => symptoms.push(
                    ImportSymptom::MissingExport {
                        folder: dir.display().to_string(),
                        export: key,
                    }
                    .into_diagnostic(span_id),
                ),
                flask::ExportResolveError::IntermediateNotFolderModule { path } => symptoms.push(
                    ImportSymptom::ChainedExportNotFolder {
                        path: path.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                ),
            }
            return Vec::new();
        }
    };

    match target {
        flask::ExportTarget::FolderModule(folder) => {
            let Some(folder_cfg) = FlaskConfig::from_directory(&folder) else {
                symptoms.push(
                    ImportSymptom::FolderMissingConfig {
                        folder: folder.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            };

            resolve_all_exports(&folder_cfg, &folder, effective_prefix, span_id, symptoms)
        }
        flask::ExportTarget::File(p) => {
            if !p.exists() {
                symptoms.push(
                    ImportSymptom::ExportTargetNotFound {
                        export: effective_prefix.to_string(),
                        folder: String::new(),
                        path: p.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return Vec::new();
            }
            vec![(p, effective_prefix.to_string())]
        }
    }
}

/// Resolve all exports from a FlaskConfig's exports map into `(path, qualifier)` pairs.
///
/// For each export, joins its path under `base_dir`, checks existence, and constructs
/// a qualifier as `{qual_prefix}.{export_key}`. Missing targets produce diagnostics
/// and are excluded from the result.
fn resolve_all_exports(
    config: &FlaskConfig,
    base_dir: &Path,
    qual_prefix: &str,
    span_id: SpanId,
    symptoms: &mut Vec<Diagnostic>,
) -> Vec<(PathBuf, String)> {
    config
        .exports()
        .iter()
        .filter_map(|(export_key, spec)| {
            let p = base_dir.join(&spec.path);
            if !p.exists() {
                symptoms.push(
                    ImportSymptom::ExportTargetNotFound {
                        export: export_key.clone(),
                        folder: base_dir.display().to_string(),
                        path: p.display().to_string(),
                    }
                    .into_diagnostic(span_id),
                );
                return None;
            }
            Some((p, format!("{qual_prefix}.{export_key}")))
        })
        .collect()
}
