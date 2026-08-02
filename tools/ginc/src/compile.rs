//! Compilation orchestration.

use crate::cli::Args;
use codegen::CodegenContext;
use diagnostic::{Category, Diagnostic, DiagnosticPathExt};
use flask::TargetTriple;
use flask::{CompileTarget, FlaskConfig};
use parser::query::SourceParseExt;
use resolve::{GinPackageExt, ParsedFile};
use std::fmt;
use std::path::{Path, PathBuf};
use typecheck::TypedFileAst;
use typecheck::compile_time_trait::CompileTimeTraitRegistry;
use typecheck::transform::{
    PackageTransformArtifacts, PackageTransformOptions, transform_package_with_shared_context,
};

struct CompilationTimings {
    enabled: bool,
    total_start: std::time::Instant,
    phases: Vec<(&'static str, std::time::Duration)>,
}

impl CompilationTimings {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            total_start: std::time::Instant::now(),
            phases: Vec::new(),
        }
    }

    fn enabled() -> bool {
        if std::env::var("GINC_TIMINGS").is_err() {
            return false;
        }

        matches!(
            std::env::var("GINC_TIMINGS")
                .map(|v| v.to_lowercase())
                .as_deref(),
            Ok("1") | Ok("true") | Ok("on") | Ok("yes")
        )
    }

    fn run<T>(&mut self, name: &'static str, action: impl FnOnce() -> T) -> T {
        let start = std::time::Instant::now();
        let value = action();
        if self.enabled {
            self.phases.push((name, start.elapsed()));
        }
        value
    }

    fn report(&self) {
        if !self.enabled {
            return;
        }

        let mut total = std::time::Duration::ZERO;
        let mut phase_parts = Vec::with_capacity(self.phases.len());
        for (name, duration) in &self.phases {
            total += *duration;
            phase_parts.push(format!("{name}:{duration:?}"));
        }

        eprintln!(
            "[ginc-timings] total:{:?} {}",
            self.total_start.elapsed(),
            phase_parts.join(" ")
        );
        if total != self.total_start.elapsed() {
            eprintln!(
                "[ginc-timings] measured_sum:{:?} total:{:?}",
                total,
                self.total_start.elapsed()
            );
        }
    }

    fn record(&mut self, name: &'static str, duration: std::time::Duration) {
        if self.enabled {
            self.phases.push((name, duration));
        }
    }
}

/// Analogous to the `ginc` command
pub struct GinCompiler;

/// Final compiler outcome.
#[derive(Debug)]
pub enum CompileResult {
    /// Compilation succeeded with no non-fatal diagnostics.
    Success { output: Option<PathBuf> },
    /// Compilation succeeded with non-fatal type-check diagnostics.
    SuccessWithTypecheckDiagnostics { output: Option<PathBuf> },
    /// Compilation failed.
    Failed(CompileFailure),
}

/// Structured fatal failure surfaced by the compiler driver.
#[derive(Debug)]
pub enum CompileFailure {
    /// No `.gin` files were discovered.
    NoInputFiles { path: PathBuf },
    /// A source file was unreadable.
    UnreadableSource { path: PathBuf, error: String },
    /// Parse stage produced a fatal flaw.
    ParseFailed,
    /// Target resolution failed.
    InvalidTarget {
        source: Option<String>,
        error: flask::TargetError,
    },
    /// Entry package did not define a concrete target.
    MissingEntryTarget,
    /// Emission or linking failed.
    EmissionFailed {
        stage: CompileEmitStage,
        diagnostics: Vec<Diagnostic>,
    },
}

/// Codegen phase names for emission failures.
#[derive(Debug)]
pub enum CompileEmitStage {
    /// Typed-AST lowering to MLIR failed.
    Codegen,
    /// MLIR → object emission failed.
    ObjectEmission,
    /// Executable linking failed.
    Linking,
}

impl fmt::Display for CompileFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoInputFiles { path } => {
                write!(f, "no .gin files found in {}", path.display())
            }
            Self::UnreadableSource { path, error } => {
                write!(f, "failed to read {}: {error}", path.display())
            }
            Self::ParseFailed => write!(f, "parse failed"),
            Self::InvalidTarget { source, error } => {
                if let Some(source) = source {
                    write!(f, "invalid target `{source}`: {error}")
                } else {
                    write!(f, "{error}")
                }
            }
            Self::MissingEntryTarget => {
                write!(f, "entry package must set `target` to a full target triple")
            }
            Self::EmissionFailed { stage, .. } => write!(f, "{stage} failed"),
        }
    }
}

impl fmt::Display for CompileEmitStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Codegen => "codegen",
            Self::ObjectEmission => "object emission",
            Self::Linking => "linking",
        };
        write!(f, "{label}")
    }
}

impl GinCompiler {
    /// Compile a Gin project through a staged pipeline.
    ///
    /// **Binary mode** (input is a `.gin` file):
    /// Imports are resolved and the result is linked into an
    /// executable (or object file / MLIR text, depending on `--emit`).
    ///
    /// **Library mode** (input is a directory):
    /// All `.gin` files are treated as a single compilation unit with a
    /// shared type environment, compiled into one object file.
    pub fn compile(args: &'_ mut Args) -> CompileResult {
        let path = args.input.to_owned();
        let mut timings = CompilationTimings::new(args.timings || CompilationTimings::enabled());

        let file_paths = timings.run("collect", || path.collect_gin_files());
        if file_paths.is_empty() {
            return CompileResult::Failed(CompileFailure::NoInputFiles { path });
        }

        let is_library = path.is_dir();
        let sources = match timings.run("collect", || read_sources(&file_paths)) {
            Ok(sources) => sources,
            Err(error) => return CompileResult::Failed(error),
        };

        let files = timings.run("parse", || parse(&sources));

        if print_diagnostics(&files) {
            return CompileResult::Failed(CompileFailure::ParseFailed);
        }

        let fast_llvm = args.fast_llvm || Self::fast_llvm_enabled();

        let entry_dir = if is_library {
            path.clone()
        } else {
            files[0]
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default()
        };
        let flask_config = FlaskConfig::find_package_config(&entry_dir).map(|(cfg, _)| cfg);
        if args.dependencies.is_empty() {
            args.dependencies = files[0].path.flask_path_dependencies();
        }

        let cli_triple = args.target.as_deref();
        let compile_target = match (flask_config.as_ref(), cli_triple) {
            (Some(config), target) => match CompileTarget::resolve(config, target) {
                Ok(compile_target) => compile_target,
                Err(error) => {
                    return CompileResult::Failed(CompileFailure::InvalidTarget {
                        source: target.map(ToString::to_string),
                        error,
                    });
                }
            },
            (None, Some(target)) => match TargetTriple::parse(target) {
                Ok(compile_target) => CompileTarget::Concrete(compile_target),
                Err(error) => {
                    return CompileResult::Failed(CompileFailure::InvalidTarget {
                        source: Some(target.to_string()),
                        error,
                    });
                }
            },
            (None, None) => CompileTarget::Library,
        };

        let is_lib_package = flask_config
            .as_ref()
            .is_none_or(|c| c.target().is_none() || c.target() == Some("library"));
        if !is_lib_package
            && cli_triple.is_none()
            && let Some(ref config) = flask_config
            && let Err(e) = config.require_entry_triple()
        {
            return CompileResult::Failed(match e {
                flask::TargetError::MissingTarget => CompileFailure::MissingEntryTarget,
                error => CompileFailure::InvalidTarget {
                    source: None,
                    error,
                },
            });
        }

        let mut files = if args.dependencies.is_empty() && !is_library {
            files
        } else {
            timings.record("inventory", std::time::Duration::from_nanos(0));
            timings.run("resolve", || {
                resolve::resolve_imports(files, &args.dependencies)
            })
        };

        if args.dependencies.is_empty() && !is_library {
            timings.run("prepare", || {
                for file in &mut files {
                    file.output.symptoms.extend(typecheck::prepare_parse_ast(
                        &mut file.output.ast,
                        &compile_target,
                    ));
                }
            });
        } else {
            let mut file_asts: Vec<ast::FileAst> =
                files.iter().map(|f| f.output.ast.clone()).collect();
            let prepare_diags: Vec<Vec<Diagnostic>> = timings.run("prepare", || {
                file_asts
                    .iter_mut()
                    .map(|ast| typecheck::prepare_parse_ast(ast, &compile_target))
                    .collect()
            });
            for (file, (ast, mut diags)) in files
                .iter_mut()
                .zip(file_asts.into_iter().zip(prepare_diags))
            {
                file.output.ast = ast;
                file.output.symptoms.append(&mut diags);
            }
        }

        if print_diagnostics(&files) {
            return CompileResult::Failed(CompileFailure::ParseFailed);
        }

        // Two-pass transform: declare all files, then lower/flow with full-package ctx
        // so defs are visible regardless of file order.
        let (typed_asts, trait_registry) = timings.run("transform/typecheck", || {
            let file_asts: Vec<ast::FileAst> = files.iter().map(|f| f.output.ast.clone()).collect();
            let PackageTransformArtifacts {
                typed_asts,
                trait_registry,
                ..
            } = transform_package_with_shared_context(file_asts, PackageTransformOptions::FULL);

            (typed_asts, trait_registry)
        });

        // Print type-check flaws from the typed AST (uses the diagnostic crate for proper
        // messages, help text, and ariadne rendering).
        //
        // Type flaws are printed but do NOT gate compilation — type checking is
        // best-effort diagnostics; codegen may still succeed for code the checker
        // doesn't fully understand yet (e.g. template unions).
        let has_typecheck_flaws =
            timings.run("typecheck", || print_type_diagnostics(&files, &typed_asts));

        let emit_result = match args.emit {
            crate::cli::Emit::Mlir => emit_mlir_typed(
                &files,
                &typed_asts,
                trait_registry.as_ref(),
                Some(&mut timings),
            ),
            crate::cli::Emit::Obj | crate::cli::Emit::Exe => emit_native_typed(
                &files,
                &typed_asts,
                NativeEmitOptions {
                    args,
                    path: &path,
                    is_library,
                    trait_registry: trait_registry.as_ref(),
                    fast_llvm,
                    timings: Some(&mut timings),
                },
            ),
        };
        timings.report();

        let output = match emit_result {
            Ok(output) => output,
            Err(failure) => return CompileResult::Failed(failure),
        };

        if has_typecheck_flaws {
            return CompileResult::SuccessWithTypecheckDiagnostics { output };
        }

        CompileResult::Success { output }
    }

    fn fast_llvm_enabled() -> bool {
        if std::env::var("GINC_FAST_LLVM").is_err() {
            return false;
        }

        matches!(
            std::env::var("GINC_FAST_LLVM")
                .map(|v| v.to_lowercase())
                .as_deref(),
            Ok("1") | Ok("true") | Ok("on") | Ok("yes")
        )
    }
}

impl CompileResult {
    pub fn emitted_path(&self) -> Option<&Path> {
        match self {
            Self::Success { output } | Self::SuccessWithTypecheckDiagnostics { output } => {
                output.as_deref()
            }
            Self::Failed(_) => None,
        }
    }

    pub fn has_typecheck_diagnostics(&self) -> bool {
        matches!(self, Self::SuccessWithTypecheckDiagnostics { .. })
    }

    pub fn is_success(&self) -> bool {
        matches!(
            self,
            Self::Success { .. } | Self::SuccessWithTypecheckDiagnostics { .. }
        )
    }
}

fn parse(sources: &[(PathBuf, String)]) -> Vec<ParsedFile> {
    sources
        .iter()
        .map(|(path, source)| {
            let output = source.parse_source_full();
            ParsedFile {
                path: path.clone(),
                source: source.clone(),
                output,
            }
        })
        .collect()
}

fn read_sources(paths: &[PathBuf]) -> Result<Vec<(PathBuf, String)>, CompileFailure> {
    let mut sources = Vec::with_capacity(paths.len());
    for fp in paths {
        match std::fs::read_to_string(fp) {
            Ok(s) => sources.push((fp.clone(), s)),
            Err(error) => {
                return Err(CompileFailure::UnreadableSource {
                    path: fp.clone(),
                    error: error.to_string(),
                });
            }
        }
    }
    Ok(sources)
}

/// Print all diagnostics for a slice of parsed files.
///
/// Each file's symptoms are printed with its own span table and source text.
/// Returns `true` if any fatal diagnostics were found.
fn print_diagnostics(files: &[ParsedFile]) -> bool {
    let mut has_flaws = false;
    for file in files {
        let filename = file.path.diagnostic_report_path_from_cwd();
        for diag in &file.output.symptoms {
            diag.print(&file.source, &filename);
            if matches!(diag.category, Category::Flaw) {
                has_flaws = true;
            }
        }
    }
    has_flaws
}

/// Print type-check flaws from the typed AST for every file.
fn print_type_diagnostics(files: &[ParsedFile], typed_asts: &[TypedFileAst]) -> bool {
    let mut has_flaws = false;
    for (file, typed) in files.iter().zip(typed_asts) {
        let filename = file.path.diagnostic_report_path_from_cwd();
        for (_, flaw) in typed.all_flaws() {
            flaw.clone().print(&file.source, &filename);
            has_flaws = true;
        }
    }
    has_flaws
}

/// Print codegen / link diagnostics with the same ariadne layout as parse and type errors.
///
/// Uses the first parsed `.gin` file as source context (span table + text). This matches
/// lowering, which is keyed off the compilation entry file.
fn print_codegen_diagnostics(files: &[ParsedFile], symptoms: &[Diagnostic]) {
    if symptoms.is_empty() {
        return;
    }
    let Some(primary) = files.first() else {
        for s in symptoms {
            eprintln!(
                "{}: [{}] {}",
                s.category.as_str(),
                s.error_code(),
                s.message
            );
        }
        return;
    };
    let label = primary.path.diagnostic_report_path_from_cwd();
    let source = primary.source.as_str();
    for d in symptoms {
        d.print(source, &label);
    }
}

/// Print MLIR text to stdout using the typed AST (no merge step).
fn emit_mlir_typed(
    files: &[ParsedFile],
    typed_asts: &[TypedFileAst],
    trait_registry: Option<&CompileTimeTraitRegistry>,
    mut timings: Option<&mut CompilationTimings>,
) -> Result<Option<PathBuf>, CompileFailure> {
    let Some(typed) = typed_asts.first() else {
        return Ok(None);
    };
    let (source, label) = match files.first() {
        Some(f) => (f.source.as_str(), f.path.diagnostic_report_path_from_cwd()),
        None => ("", "<stdin>".to_string()),
    };
    let context = codegen::emit::NativeCompiler::create_context();
    let (module, symptoms) = if let Some(timings) = timings.as_mut() {
        timings.run("mlir-lowering", || {
            CodegenContext::build_module_from_typed_ast(
                &context,
                typed,
                source,
                &label,
                trait_registry,
            )
        })
    } else {
        CodegenContext::build_module_from_typed_ast(&context, typed, source, &label, trait_registry)
    };
    let result = module.map(|m| m.as_operation().to_string());
    match result {
        Some(mlir_text) => {
            print_codegen_diagnostics(files, &symptoms);
            println!("\n```mlir\n{mlir_text}```\n");
            Ok(None)
        }
        None => {
            print_codegen_diagnostics(files, &symptoms);
            Err(CompileFailure::EmissionFailed {
                stage: CompileEmitStage::Codegen,
                diagnostics: symptoms,
            })
        }
    }
}

/// Compile to object file / executable using the typed AST (no merge step).
struct NativeEmitOptions<'a> {
    args: &'a Args,
    path: &'a Path,
    is_library: bool,
    trait_registry: Option<&'a CompileTimeTraitRegistry>,
    fast_llvm: bool,
    timings: Option<&'a mut CompilationTimings>,
}

fn emit_native_typed(
    files: &[ParsedFile],
    typed_asts: &[TypedFileAst],
    options: NativeEmitOptions<'_>,
) -> Result<Option<PathBuf>, CompileFailure> {
    let NativeEmitOptions {
        args,
        path,
        is_library,
        trait_registry,
        fast_llvm,
        mut timings,
    } = options;
    let (Some(typed), Some(file)) = (typed_asts.first(), files.first()) else {
        return Ok(None);
    };
    let obj_path = if is_library {
        // Folder packages reuse `args.output` only for `-o exe`/`link` destinations.
        // When emitting Exe we always stage `.o` under `<pkg>/target/` so `-o foo`
        // is never scribbled onto as raw object contents (which corrupts linkage).
        if matches!(args.emit, crate::cli::Emit::Exe) {
            let pkg_name = path.file_name().unwrap_or_default().to_string_lossy();
            path.join("target").join(format!("{pkg_name}.o"))
        } else {
            args.output.clone().unwrap_or_else(|| {
                let pkg_name = path.file_name().unwrap_or_default().to_string_lossy();
                path.join("target").join(format!("{}.o", pkg_name))
            })
        }
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

    if let Some(parent) = obj_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let source = file.source.as_str();
    let label = file.path.diagnostic_report_path_from_cwd();
    let profile = args.profile;

    let compiler = codegen::emit::NativeCompiler::default();
    let context = codegen::emit::NativeCompiler::create_context();
    let (module, symptoms) = if let Some(timings) = timings.as_mut() {
        timings.run("mlir-lowering", || {
            CodegenContext::build_module_from_typed_ast(
                &context,
                typed,
                source,
                &label,
                trait_registry,
            )
        })
    } else {
        CodegenContext::build_module_from_typed_ast(&context, typed, source, &label, trait_registry)
    };
    let Some(module) = module else {
        print_codegen_diagnostics(files, &symptoms);
        return Err(CompileFailure::EmissionFailed {
            stage: CompileEmitStage::Codegen,
            diagnostics: symptoms,
        });
    };
    let (ok, more, native_timings) =
        compiler.native_from_module_with_timings(&module, &obj_path, profile, fast_llvm);
    if let Some(timings) = timings.as_mut() {
        timings.record("mlir-optimization", native_timings.optimization);
        timings.record("llvm-lowering", native_timings.lowering);
        timings.record("object-emission", native_timings.object_emission);
    }
    let mut symptoms = symptoms;
    symptoms.extend(more);
    if !ok {
        print_codegen_diagnostics(files, &symptoms);
        return Err(CompileFailure::EmissionFailed {
            stage: CompileEmitStage::ObjectEmission,
            diagnostics: symptoms,
        });
    }

    if matches!(args.emit, crate::cli::Emit::Exe) {
        let exe_path = args
            .output
            .clone()
            .unwrap_or_else(|| path.with_extension(""));
        let (linked, link_symptoms) = if let Some(timings) = timings.as_mut() {
            timings.run("linking", || {
                codegen::emit::NativeCompiler::link_executable(
                    &obj_path,
                    &exe_path,
                    args.target.as_deref(),
                )
            })
        } else {
            codegen::emit::NativeCompiler::link_executable(
                &obj_path,
                &exe_path,
                args.target.as_deref(),
            )
        };
        if !linked {
            print_codegen_diagnostics(files, &link_symptoms);
            return Err(CompileFailure::EmissionFailed {
                stage: CompileEmitStage::Linking,
                diagnostics: link_symptoms,
            });
        }
        let _ = std::fs::remove_file(&obj_path);
        Ok(Some(exe_path))
    } else {
        println!("Compiled to {}", obj_path.display());
        Ok(Some(obj_path))
    }
}
