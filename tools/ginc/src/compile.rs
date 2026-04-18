//! Compilation orchestration — the main compiler pipeline.
//!

// TODO: Separate compilation for modules — currently all files are merged into one AST
// before codegen. A better approach is to compile module files independently into object
// files with global symbol tables, then link them. This keeps user code and library code
// distinct for easier debugging and enables incremental recompilation.

use crate::cli::Args;
use ast::ImportSource;
use ast::{FileAst, qualify_module_defs};
use codegen::emit::native;
use diagnostic::lex::LexSymptom;
use diagnostic::parse::ParseSymptom;
use diagnostic::{Category, Symptom, SymptomLike};
use flask::{DependencyKind, FlaskConfig};
use lexer::debug_tokens;
use parser::{ParseOutput, discover_module, extract_package_import_paths, parse_source_full};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use typeck::{TyEnv, analyze_file};

/// Holds parsed file data alongside its source for diagnostic reporting.
struct ParsedFile {
    path: PathBuf,
    source: String,
    output: ParseOutput,
}

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
        if !is_library && !parsed_files.is_empty() {
            // Extract data from the entry file before mutating parsed_files.
            let entry_path = parsed_files[0].path.clone();
            let entry_dir = entry_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let entry_ast = parsed_files[0].output.ast.clone();
            let base_dir = entry_path.parent().unwrap_or(Path::new(""));

            if args.dependencies.is_empty() {
                if let Some(config) = FlaskConfig::from_directory(&entry_dir) {
                    args.dependencies = resolve_flask_path_dependencies(&config, &entry_dir);
                }
            }

            if !args.dependencies.is_empty() {
                let mut dep_paths: Vec<PathBuf> = args
                    .dependencies
                    .values()
                    .flat_map(|root| collect_gin_files_recursive(root))
                    .collect();
                dep_paths.sort();
                let mut insert_idx = 1usize;
                for file_path in dep_paths {
                    if parsed_files.iter().any(|p| p.path == file_path) {
                        continue;
                    }
                    let source = match std::fs::read_to_string(&file_path) {
                        Ok(s) => s,
                        Err(err) => {
                            eprintln!("Error reading dependency {}: {}", file_path.display(), err);
                            continue;
                        }
                    };
                    let mut output = parse_source_full(&source);
                    if let Some(qual) =
                        module_qualifier_for_dep_file(&file_path, &args.dependencies)
                    {
                        output.ast = qualify_module_defs(output.ast, &qual);
                    }
                    parsed_files.insert(
                        insert_idx,
                        ParsedFile {
                            path: file_path,
                            source,
                            output,
                        },
                    );
                    insert_idx += 1;
                }
            }

            // Local imports: use module tree discovery to find direct files
            // and all sub-module files. `use 'utils'` pulls in utils/*.gin for
            // unqualified access AND utils/requests/*.gin etc. so that qualified
            // calls like `requests.make_request(...)` can resolve.
            for import in entry_ast.uses() {
                for module_import in &import.0 {
                    if let ImportSource::Local(path, _span) = &module_import.source {
                        let import_dir = base_dir.join(path);
                        let Some(tree) = discover_module(&import_dir) else {
                            continue;
                        };

                        for file_path in tree.all_files_recursive() {
                            if parsed_files.iter().any(|p| p.path == file_path) {
                                continue;
                            }
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
                            let output = parse_source_full(&source);
                            parsed_files.push(ParsedFile {
                                path: file_path,
                                source,
                                output,
                            });
                        }
                    }
                }
            }

            // Package imports: resolve against flask.json dependency map.
            let pkg_paths = extract_package_import_paths(&entry_ast, &args.dependencies);
            for (import_path, _span) in &pkg_paths {
                if parsed_files.iter().any(|p| p.path == *import_path) {
                    continue;
                }
                let source = match std::fs::read_to_string(import_path) {
                    Ok(s) => s,
                    Err(err) => {
                        eprintln!("Error reading import {}: {}", import_path.display(), err);
                        continue;
                    }
                };
                let output = parse_source_full(&source);
                parsed_files.push(ParsedFile {
                    path: import_path.clone(),
                    source,
                    output,
                });
            }
        }

        // ── Phase 4: Build shared AST list for analysis ──────────────
        let all_asts: Vec<FileAst> = parsed_files.iter().map(|p| p.output.ast.clone()).collect();

        // ── Phase 5: Print diagnostics ────────────────────────────────
        let has_flaws = print_diagnostics(&parsed_files, &all_asts);
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

fn module_qualifier_for_dep_file(path: &Path, deps: &HashMap<String, PathBuf>) -> Option<String> {
    let file = std::fs::canonicalize(path).ok()?;
    for dep_root in deps.values() {
        let root = std::fs::canonicalize(dep_root).ok()?;
        if let Ok(rel) = file.strip_prefix(&root) {
            let mut s = rel.to_string_lossy().replace('\\', "/");
            s = s.trim_start_matches('/').to_string();
            if let Some(stripped) = s.strip_suffix(".gin") {
                s = stripped.to_string();
            }
            if s.is_empty() {
                return None;
            }
            return Some(s.replace('/', "."));
        }
    }
    None
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
fn print_diagnostics(parsed_files: &[ParsedFile], all_asts: &[FileAst]) -> bool {
    let mut has_flaws = false;

    for (i, parsed) in parsed_files.iter().enumerate() {
        let filename = parsed.filename();
        let span_table = &parsed.output.span_table;
        let mut symptoms: Vec<Symptom> = Vec::new();

        // Unterminated strings
        for &span_id in &parsed.output.unterminated_strings {
            symptoms.push(LexSymptom::UnclosedString.into_symptom(span_id));
        }

        // Lex errors
        for (symptom, span_id) in &parsed.output.lex_errors {
            symptoms.push(symptom.clone().into_symptom(*span_id));
        }

        // Parse errors
        for err in &parsed.output.parse_errors {
            symptoms.push(ParseSymptom::Custom(err.message.clone()).into_symptom(err.span));
        }

        // Help hints (empty-paren suggestions)
        for (suggested, span_id) in &parsed.output.help_hints {
            symptoms.push(
                ParseSymptom::EmptyParens {
                    suggested: suggested.clone(),
                }
                .into_symptom(*span_id),
            );
        }

        // Unused value info diagnostics
        for (value, span_id) in &parsed.output.unused_values {
            symptoms.push(
                ParseSymptom::UnusedValue {
                    value: value.clone(),
                }
                .into_symptom(*span_id),
            );
        }

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
                eprintln!("Codegen warning: [{}] {}", s.code, s.message);
            }
            println!("\n```mlir\n{mlir_text}```\n");
        }
        None => {
            for s in &symptoms {
                eprintln!("Codegen error: [{}] {}", s.code, s.message);
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
            eprintln!("Codegen error: [{}] {}", s.code, s.message);
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
                eprintln!("Link error: [{}] {}", s.code, s.message);
            }
        }
        let _ = std::fs::remove_file(&obj_path);
    } else {
        for s in &symptoms {
            eprintln!("Codegen warning: [{}] {}", s.code, s.message);
        }
        println!("Compiled to {}", obj_path.display());
    }
}
