//! Compilation orchestration.

use crate::cli::Args;
use crate::driver::{
    CheckedFile, CheckedPackage, CompilePackage, CompilerDriver, CompilerDriverError,
    LoweredFileDiagnostics, ParsedPackage, SourceFile, SourcePackage,
};
use diagnostic::{Category, Diagnostic, DiagnosticPathExt};
use flask::TargetTriple;
use flask::{CompileTarget, FlaskConfig};
use resolve::GinPackageExt;
use std::path::{Path, PathBuf};
use strum::Display;
use thiserror::Error;

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
        env_flag_enabled("GINC_TIMINGS")
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
#[derive(Debug, Error)]
pub enum CompileFailure {
    /// No `.gin` files were discovered.
    #[error("no .gin files found in {path}")]
    NoInputFiles { path: PathBuf },
    /// A source file was unreadable.
    #[error("failed to read {path}: {error}")]
    UnreadableSource { path: PathBuf, error: String },
    /// Parse stage produced a fatal flaw.
    #[error("parse failed")]
    ParseFailed,
    /// Compiler input violated a structural driver invariant.
    #[error("compiler driver failed: {0}")]
    Driver(CompilerDriverError),
    /// Target resolution failed.
    #[error("{0}")]
    InvalidTarget(flask::TargetError),
    /// Entry package did not define a concrete target.
    #[error("entry package must set `target` to a full target triple")]
    MissingEntryTarget,
    /// Emission or linking failed.
    #[error("{stage} failed")]
    EmissionFailed {
        stage: CompileEmitStage,
        diagnostics: Vec<Diagnostic>,
    },
}

/// Codegen phase names for emission failures.
#[derive(Debug, Display)]
pub enum CompileEmitStage {
    /// Typed-AST lowering to MLIR failed.
    #[strum(to_string = "codegen")]
    Codegen,
    /// MLIR → object emission failed.
    #[strum(to_string = "object emission")]
    ObjectEmission,
    /// Executable linking failed.
    #[strum(to_string = "linking")]
    Linking,
    /// Interface artifact emission failed.
    #[strum(to_string = "interface emission")]
    InterfaceEmission,
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
        let primary_path = sources[0].path.clone();
        let mut driver = CompilerDriver::default();
        let parsed = match timings.run("parse", || {
            driver.parse(SourcePackage::new(primary_path.clone(), sources))
        }) {
            Ok(parsed) => parsed,
            Err(error) => return CompileResult::Failed(CompileFailure::Driver(error)),
        };
        if print_parse_diagnostics(&parsed) {
            return CompileResult::Failed(CompileFailure::ParseFailed);
        }

        let fast_llvm = args.fast_llvm || Self::fast_llvm_enabled();

        let entry_dir = if is_library {
            path.clone()
        } else {
            primary_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default()
        };
        let flask_config = FlaskConfig::find_package_config(&entry_dir).map(|(cfg, _)| cfg);
        if args.dependencies.is_empty() {
            args.dependencies = primary_path.flask_path_dependencies();
        }

        let cli_triple = args.target.as_deref();
        let compile_target = match (flask_config.as_ref(), cli_triple) {
            (Some(config), target) => match CompileTarget::resolve(config, target) {
                Ok(compile_target) => compile_target,
                Err(error) => {
                    return CompileResult::Failed(match target {
                        Some(_) | None => CompileFailure::InvalidTarget(error),
                    });
                }
            },
            (None, Some(target)) => match TargetTriple::parse(target) {
                Ok(compile_target) => CompileTarget::Concrete(compile_target),
                Err(error) => {
                    return CompileResult::Failed(CompileFailure::InvalidTarget(error));
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
                error => CompileFailure::InvalidTarget(error),
            });
        }

        let use_package_resolver =
            flask_config.is_some() || !args.dependencies.is_empty() || is_library;
        let checked = timings.run("front-end", || {
            driver.check(
                CompilePackage::new(parsed, compile_target)
                    .with_dependencies(args.dependencies.clone())
                    .with_import_resolution(use_package_resolver),
            )
        });

        if print_diagnostics(&checked) {
            return CompileResult::Failed(CompileFailure::ParseFailed);
        }

        // Print type-check flaws from the typed AST (uses the diagnostic crate for proper
        // messages, help text, and ariadne rendering).
        //
        // Type flaws are printed but do NOT gate compilation — type checking is
        // best-effort diagnostics; codegen may still succeed for code the checker
        // doesn't fully understand yet (e.g. template unions).
        let has_typecheck_flaws = timings.run("typecheck", || print_type_diagnostics(&checked));

        let emit_result = match args.emit {
            crate::cli::Emit::Mlir => emit_mlir_typed(&driver, &checked, Some(&mut timings)),
            crate::cli::Emit::Interface => emit_interface(&checked, args),
            crate::cli::Emit::Obj | crate::cli::Emit::Exe => emit_native_typed(
                &driver,
                &checked,
                NativeEmitOptions {
                    args,
                    path: &path,
                    is_library,
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
        env_flag_enabled("GINC_FAST_LLVM")
    }
}

fn env_flag_enabled(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .map(|value| value.to_lowercase())
            .as_deref(),
        Ok("1") | Ok("true") | Ok("on") | Ok("yes")
    )
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

fn read_sources(paths: &[PathBuf]) -> Result<Vec<SourceFile>, CompileFailure> {
    let mut sources = Vec::with_capacity(paths.len());
    for fp in paths {
        match std::fs::read_to_string(fp) {
            Ok(source) => sources.push(SourceFile::new(fp.clone(), source)),
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
fn print_parse_diagnostics(package: &ParsedPackage) -> bool {
    let mut has_flaws = false;
    for file in package.files() {
        has_flaws |= print_file_diagnostics(&file.path, &file.source, &file.output.symptoms);
    }
    has_flaws
}

fn print_diagnostics(package: &CheckedPackage) -> bool {
    let mut has_flaws = false;
    for file in package.files() {
        has_flaws |=
            print_file_diagnostics(file.path(), file.source(), file.front_end_diagnostics());
    }
    has_flaws
}

fn print_file_diagnostics(path: &Path, source: &str, diagnostics: &[Diagnostic]) -> bool {
    let filename = path.diagnostic_report_path_from_cwd();
    for diagnostic in diagnostics {
        diagnostic.print(source, &filename);
    }
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.category == Category::Flaw)
}

/// Print type-check flaws from the typed AST for every file.
fn print_type_diagnostics(package: &CheckedPackage) -> bool {
    let mut has_flaws = false;
    for file in package.files() {
        let filename = file.path().diagnostic_report_path_from_cwd();
        for flaw in file.typecheck_diagnostics() {
            flaw.clone().print(file.source(), &filename);
            has_flaws = true;
        }
    }
    has_flaws
}

/// Backend and linker diagnostics lack file ownership, so they use the primary source context.
fn print_codegen_diagnostics(primary: &CheckedFile, symptoms: &[Diagnostic]) {
    if symptoms.is_empty() {
        return;
    }
    let label = primary.path().diagnostic_report_path_from_cwd();
    let source = primary.source();
    for d in symptoms {
        d.print(source, &label);
    }
}

fn print_package_codegen_diagnostics(
    package: &CheckedPackage,
    diagnostics: &[LoweredFileDiagnostics],
) {
    for file_diagnostics in diagnostics {
        let file = package
            .file(&file_diagnostics.path)
            .expect("codegen diagnostics retain their checked source");
        print_file_diagnostics(file.path(), file.source(), &file_diagnostics.diagnostics);
    }
}

fn emit_mlir_typed(
    driver: &CompilerDriver,
    package: &CheckedPackage,
    mut timings: Option<&mut CompilationTimings>,
) -> Result<Option<PathBuf>, CompileFailure> {
    let context = codegen::emit::NativeCompiler::create_context();
    let lowered = if let Some(timings) = timings.as_mut() {
        timings.run("mlir-lowering", || driver.lower_package(&context, package))
    } else {
        driver.lower_package(&context, package)
    };
    print_package_codegen_diagnostics(package, &lowered.diagnostics);
    let symptoms = lowered
        .diagnostics
        .into_iter()
        .flat_map(|file| file.diagnostics)
        .collect();
    let result = lowered
        .module
        .map(|module| module.as_operation().to_string());
    match result {
        Some(mlir_text) => {
            println!("\n```mlir\n{mlir_text}```\n");
            Ok(None)
        }
        None => Err(CompileFailure::EmissionFailed {
            stage: CompileEmitStage::Codegen,
            diagnostics: symptoms,
        }),
    }
}

struct NativeEmitOptions<'a> {
    args: &'a Args,
    path: &'a Path,
    is_library: bool,
    fast_llvm: bool,
    timings: Option<&'a mut CompilationTimings>,
}

fn emit_native_typed(
    driver: &CompilerDriver,
    package: &CheckedPackage,
    options: NativeEmitOptions<'_>,
) -> Result<Option<PathBuf>, CompileFailure> {
    let NativeEmitOptions {
        args,
        path,
        is_library,
        fast_llvm,
        mut timings,
    } = options;
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

    let profile = args.profile;

    let compiler = codegen::emit::NativeCompiler::default();
    let context = codegen::emit::NativeCompiler::create_context();
    let lowered = if let Some(timings) = timings.as_mut() {
        timings.run("mlir-lowering", || driver.lower_package(&context, package))
    } else {
        driver.lower_package(&context, package)
    };
    print_package_codegen_diagnostics(package, &lowered.diagnostics);
    let mut symptoms: Vec<_> = lowered
        .diagnostics
        .into_iter()
        .flat_map(|file| file.diagnostics)
        .collect();
    let Some(module) = lowered.module else {
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
    if !ok {
        print_codegen_diagnostics(package.primary_file(), &more);
        symptoms.extend(more);
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
            print_codegen_diagnostics(package.primary_file(), &link_symptoms);
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

fn emit_interface(
    package: &CheckedPackage,
    args: &Args,
) -> Result<Option<PathBuf>, CompileFailure> {
    let is_library = args.input.is_dir();
    let interface_path = if is_library {
        let pkg_name = args.input.file_name().unwrap_or_default().to_string_lossy();
        args.output
            .clone()
            .unwrap_or_else(|| args.input.join("target").join(format!("{pkg_name}.ginif")))
    } else {
        args.output
            .clone()
            .unwrap_or_else(|| args.input.with_extension("ginif"))
    };

    if let Some(parent) = interface_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let temp_path = interface_path.with_extension("ginif.tmp");
    if let Err(error) = std::fs::write(&temp_path, package.public_interface().canonical_bytes()) {
        return Err(CompileFailure::EmissionFailed {
            stage: CompileEmitStage::InterfaceEmission,
            diagnostics: vec![Diagnostic::new(
                "interface-write-failed",
                format!("failed to write temporary interface artifact: {error}"),
            )],
        });
    }
    if let Err(error) = std::fs::rename(&temp_path, &interface_path) {
        let cleanup = std::fs::remove_file(&temp_path);
        if cleanup.is_err() {
            // Ignore cleanup failures; keep the primary error as authoritative.
        }
        return Err(CompileFailure::EmissionFailed {
            stage: CompileEmitStage::InterfaceEmission,
            diagnostics: vec![Diagnostic::new(
                "interface-write-failed",
                format!("failed to publish interface artifact: {error}"),
            )],
        });
    }

    println!("Wrote interface artifact {}", interface_path.display());
    Ok(Some(interface_path))
}
