//! Compilation orchestration — the main compiler pipeline.
//!

// TODO: Separate compilation for modules — currently all files are merged into one AST
// before codegen. A better approach is to compile module files independently into object
// files with global symbol tables, then link them. This keeps user code and library code
// distinct for easier debugging and enables incremental recompilation.

use crate::cli::Args;
use ast::ImportSource;
use ast::{FileAst, ModPath, ModuleImport, qualify_module_defs};
use codegen::emit::native;
use diagnostic::{Category, Diagnostic, DiagnosticLike, ImportSymptom, SpanId};
use flask::{DependencyKind, FlaskConfig};
use lexer::debug_tokens;
use parser::{ParseOutput, parse_source_full};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use typeck::{TyEnv, analyze_file};

/// Holds parsed file data alongside its source for diagnostic reporting.
struct ParsedFile {
    path: PathBuf,
    source: String,
    output: ParseOutput,
}

// TODO: can we just use the ParseOutput for this?
// TODO: simplify all this file/improve perf

impl ParsedFile {
    fn filename(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

/// Analogous to the `ginc` command
pub struct GinCompiler;

impl GinCompiler {
    /// Compile a Gin project through a unified pipeline.
    ///
    /// **Binary mode** (input is a `.gin` file):
    /// Imports are resolved and the result is linked into an
    /// executable (or object file / MLIR text, depending on `--emit`).
    ///
    /// **Library mode** (input is a directory):
    /// All `.gin` files are treated as a single compilation unit with a
    /// shared type environment, compiled into one object file.
    pub fn compile(args: &'_ mut Args) {
        let path = args.input.to_owned();
        let is_library = path.is_dir();

        // ── Phase 1: Collect source file paths ────────────────────────
        let file_paths = if is_library {
            collect_gin_files_recursive(&path)
        } else {
            vec![path.clone()]
        };

        if file_paths.is_empty() {
            eprintln!("No .gin files found in {}", path.display());
            return;
        }

        // ── Phase 2: Read and parse all files ─────────────────────────
        let mut parsed_files: Vec<ParsedFile> = Vec::with_capacity(file_paths.len());
        for fp in &file_paths {
            let source = match std::fs::read_to_string(fp) {
                Ok(s) => s,
                Err(err) => {
                    eprintln!("Error reading {}: {}", fp.display(), err);
                    return;
                }
            };
            let output = parse_source_full(&source);
            parsed_files.push(ParsedFile {
                path: fp.clone(),
                source,
                output,
            });
        }

        if matches!(args.emit, crate::cli::Emit::Tokens) {
            for parsed in &parsed_files {
                print!("{}", debug_tokens(&parsed.source));
            }
            return;
        }

        // ── Phase 3: Resolve imports (binary mode only) ──────────────
        // Extract data from entry file before mutating parsed_files.
        let entry_path = parsed_files[0].path.clone();
        let entry_dir = entry_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let entry_ast = parsed_files[0].output.ast.clone();

        if !is_library && !parsed_files.is_empty() {
            if args.dependencies.is_empty()
                && let Some(config) = FlaskConfig::from_directory(&entry_dir)
            {
                args.dependencies = resolve_flask_path_dependencies(&config, &entry_dir);
            }

            let mut seen: HashMap<PathBuf, String> = HashMap::new();
            let mut node_by_path: HashMap<PathBuf, usize> = HashMap::new();
            node_by_path.insert(entry_path.clone(), 0);
            seen.insert(entry_path.clone(), String::new());

            // Adjacency list of the module graph (one node per parsed file, by index).
            let mut adj: Vec<Vec<crate::module_graph::ImportEdge>> = vec![Vec::new()];
            let mut processed_imports: Vec<bool> = vec![false];

            // Recursive import expansion: as we discover files, parse them and then walk their imports.
            loop {
                let next = processed_imports
                    .iter()
                    .enumerate()
                    .find_map(|(i, done)| (!done).then_some(i));
                let Some(from_idx) = next else { break };
                processed_imports[from_idx] = true;

                let from_path = parsed_files[from_idx].path.clone();
                let from_dir = from_path.parent().unwrap_or(Path::new("")).to_path_buf();
                let from_ast = parsed_files[from_idx].output.ast.clone();

                for import in from_ast.uses() {
                    for module_import in &import.0 {
                        let span_id = ast::HasSpanId::span_id(module_import);
                        let mut import_symptoms: Vec<Diagnostic> = Vec::new();
                        let resolved = resolve_module_import(
                            module_import,
                            &from_dir,
                            &args.dependencies,
                            span_id,
                            &mut import_symptoms,
                        );
                        parsed_files[from_idx]
                            .output
                            .symptoms
                            .extend(import_symptoms);

                        for (file_path, qual) in resolved {
                            if file_path == entry_path {
                                // Self-import is still an edge; it forms a trivial cycle.
                            }

                            if !file_path.is_file() {
                                parsed_files[from_idx].output.symptoms.push(
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
                                parsed_files[from_idx].output.symptoms.push(
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

                                let i = parsed_files.len();
                                parsed_files.push(ParsedFile {
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

            // Detect cycles in the discovered graph.
            if let Some(cycle) = crate::module_graph::detect_first_cycle(&adj) {
                // Format as a path chain for help text.
                let mut parts: Vec<String> = Vec::new();
                for &n in &cycle.nodes {
                    parts.push(parsed_files[n].path.display().to_string());
                }
                let chain = parts.join(" -> ");

                parsed_files[cycle.closing_from]
                    .output
                    .symptoms
                    .push(ImportSymptom::Cycle { chain }.into_diagnostic(cycle.closing_span));
            }
        }

        // ── Phase 4: Build shared AST list for analysis ──────────────
        let all_asts: Vec<FileAst> = parsed_files.iter().map(|p| p.output.ast.clone()).collect();

        // ── Phase 5: Print diagnostics ────────────────────────────────
        let has_flaws = print_diagnostics(&parsed_files, &all_asts, &entry_ast);
        if has_flaws {
            return;
        }

        // ── Phase 6: Build merged AST for codegen ─────────────────────
        let mut merged_ast = FileAst::default();
        for ast in &all_asts {
            merged_ast.merge_from(ast.clone());
        }
        let ty_env = TyEnv::from_file_ast(&merged_ast);

        // ── Phase 7: Emit output ──────────────────────────────────────
        match args.emit {
            crate::cli::Emit::Mlir => emit_mlir(&merged_ast, &ty_env),
            crate::cli::Emit::Obj | crate::cli::Emit::Exe => {
                emit_native(&merged_ast, &ty_env, args, &path, is_library)
            }
            crate::cli::Emit::Tokens => unreachable!(),
        }
    }
}

// ── File Collection ─────────────────────────────────────────────────────────

/// Collect all .gin file paths in a directory recursively, skipping `target/`.
fn resolve_flask_path_dependencies(
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

/// Returns `(absolute_path, qualifier_prefix)` for each `.gin` file to merge.
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
    mp: &ModPath,
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

            // Chained exports for local folder-modules (root name is local folder).
            // - `use utils` imports all exports from `utils/flask.jsonc`
            // - `use utils.a.b` walks exports a -> (folder-module) then b -> target
            if mp.segments.is_empty() {
                return config
                    .exports()
                    .iter()
                    .map(|(export_key, spec)| {
                        let p = folder.join(&spec.path);
                        if !p.exists() {
                            symptoms.push(
                                ImportSymptom::ExportTargetNotFound {
                                    export: export_key.clone(),
                                    folder: folder.display().to_string(),
                                    path: p.display().to_string(),
                                }
                                .into_diagnostic(span_id),
                            );
                            return (PathBuf::new(), String::new());
                        }
                        let qual = format!("{eff_root}.{export_key}");
                        (p, qual)
                    })
                    .filter(|(p, q)| !p.as_os_str().is_empty() && !q.is_empty())
                    .collect();
            }

            // For chained imports that resolve to folder-modules, exports must be qualified
            // with the full chain prefix, e.g. `use utils.a` -> `utils.a.<export>`.
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

            // Package imports resolve through exports only, supporting chained exports:
            // - `use dep` imports all exports of dep root
            // - `use dep.a.b` walks `exports[a]` (must be folder-module unless last) then `exports[b]`, etc.
            if mp.segments.is_empty() {
                return config
                    .exports()
                    .iter()
                    .map(|(export_key, spec)| {
                        let p = dep_dir.join(&spec.path);
                        if !p.exists() {
                            symptoms.push(
                                ImportSymptom::ExportTargetNotFound {
                                    export: export_key.clone(),
                                    folder: dep_dir.display().to_string(),
                                    path: p.display().to_string(),
                                }
                                .into_diagnostic(span_id),
                            );
                            return (PathBuf::new(), String::new());
                        }
                        let qual = match &module_import.alias {
                            Some(a) => format!("{}.{}", a, export_key),
                            None => export_key.clone(),
                        };
                        (p, qual)
                    })
                    .filter(|(p, q)| !p.as_os_str().is_empty() && !q.is_empty())
                    .collect();
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
    segments: &[internment::Intern<String>],
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

            folder_cfg
                .exports()
                .iter()
                .filter_map(|(export_key, spec)| {
                    let p = folder.join(&spec.path);
                    if !p.exists() {
                        symptoms.push(
                            ImportSymptom::ExportTargetNotFound {
                                export: export_key.clone(),
                                folder: folder.display().to_string(),
                                path: p.display().to_string(),
                            }
                            .into_diagnostic(span_id),
                        );
                        return None;
                    }
                    Some((p, format!("{effective_prefix}.{export_key}")))
                })
                .collect()
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

// ── Diagnostics ─────────────────────────────────────────────────────────────

/// Print diagnostics for all files — parse errors, lex errors, and type/flow analysis.
///
/// Returns `true` if any fatal flaws were found.
fn print_diagnostics(
    parsed_files: &[ParsedFile],
    all_asts: &[FileAst],
    _entry_ast: &FileAst,
) -> bool {
    let mut has_flaws = false;

    for (i, parsed) in parsed_files.iter().enumerate() {
        let filename = parsed.filename();
        let span_table = parsed.output.ast.span_table();

        let mut symptoms = parsed.output.symptoms.clone();

        // Type-check and flow-analysis symptoms
        symptoms.extend(analyze_file(&all_asts[i], all_asts));

        // Print all symptoms for this file
        for symptom in &symptoms {
            symptom.print(span_table, &parsed.source, &filename);
            if matches!(symptom.category, Category::Flaw) {
                has_flaws = true;
            }
        }
    }

    has_flaws
}

// ── Codegen & Output ────────────────────────────────────────────────────────

/// Print MLIR text to stdout.
fn emit_mlir(merged_ast: &FileAst, ty_env: &TyEnv) {
    let (result, symptoms) = native::build_module_text(merged_ast, "", "<stdin>", ty_env);
    match result {
        Some(mlir_text) => {
            for s in &symptoms {
                eprintln!("Codegen warning: [{}] {}", s.error_code(), s.message);
            }
            println!("\n```mlir\n{mlir_text}```\n");
        }
        None => {
            for s in &symptoms {
                eprintln!("Codegen error: [{}] {}", s.error_code(), s.message);
            }
        }
    }
}

/// Compile to object file / executable.
fn emit_native(merged_ast: &FileAst, ty_env: &TyEnv, args: &Args, path: &Path, is_library: bool) {
    // Determine object file path
    let obj_path = if is_library {
        args.output.clone().unwrap_or_else(|| {
            let pkg_name = path.file_name().unwrap_or_default().to_string_lossy();
            path.join("target").join(format!("{}.o", pkg_name))
        })
    } else if matches!(args.emit, crate::cli::Emit::Exe) {
        args.output
            .clone()
            .unwrap_or_else(|| path.with_extension(""))
            .with_extension("o")
    } else {
        args.output
            .clone()
            .unwrap_or_else(|| path.with_extension("o"))
    };

    // Ensure target directory exists
    if let Some(parent) = obj_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let filename = path.to_string_lossy();
    let profile = args.profile.into();
    let (ok, symptoms) =
        native::compile_to_object(merged_ast, &obj_path, profile, "", &filename, ty_env);
    if !ok {
        for s in &symptoms {
            eprintln!("Codegen error: [{}] {}", s.error_code(), s.message);
        }
        return;
    }

    if matches!(args.emit, crate::cli::Emit::Exe) {
        let exe_path = args
            .output
            .clone()
            .unwrap_or_else(|| path.with_extension(""));
        let (linked, link_symptoms) =
            native::link_executable(&obj_path, &exe_path, args.target.as_deref());
        if !linked {
            for s in &link_symptoms {
                eprintln!("Link error: [{}] {}", s.error_code(), s.message);
            }
        }
        let _ = std::fs::remove_file(&obj_path);
    } else {
        for s in &symptoms {
            eprintln!("Codegen warning: [{}] {}", s.error_code(), s.message);
        }
        println!("Compiled to {}", obj_path.display());
    }
}
