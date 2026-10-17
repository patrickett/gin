//! Compilation orchestration.

use crate::cli::Args;
use crate::driver::{
    CheckedFile, CheckedPackage, CompilePackage, CompilerDriver, CompilerDriverError,
    LoweredFileDiagnostics, ParsedPackage, SourceFile, SourcePackage,
};
use crate::reporting::{CheckOptions, DiagnosticReporter};
use diagnostic::code::InterfaceCode;
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
    json_path: Option<PathBuf>,
    object_cache_hit: Option<bool>,
}

impl CompilationTimings {
    fn new(enabled: bool, json_path: Option<PathBuf>) -> Self {
        Self {
            enabled,
            total_start: std::time::Instant::now(),
            phases: Vec::new(),
            json_path,
            object_cache_hit: None,
        }
    }

    fn enabled() -> bool {
        env_flag_enabled("GINC_TIMINGS")
    }

    fn run<T>(&mut self, name: &'static str, action: impl FnOnce() -> T) -> T {
        let phase = match name {
            "discover-sources" => compiler_profile::Phase::Discover,
            "read-sources" => compiler_profile::Phase::Read,
            "parse" => compiler_profile::Phase::Parse,
            "front-end" => compiler_profile::Phase::FrontEnd,
            "typecheck-diagnostics" => compiler_profile::Phase::Diagnostics,
            "mlir-lowering" => compiler_profile::Phase::Mlir,
            "artifact-key" | "toolchain-identity" => compiler_profile::Phase::ArtifactKey,
            "object-cache-lookup" => compiler_profile::Phase::ObjectCache,
            "linking" => compiler_profile::Phase::Linking,
            _ => compiler_profile::Phase::Other,
        };
        let _profile = compiler_profile::span(phase);
        if !self.enabled && self.json_path.is_none() {
            return action();
        }
        let start = std::time::Instant::now();
        let value = action();
        if self.enabled || self.json_path.is_some() {
            self.phases.push((name, start.elapsed()));
        }
        value
    }

    fn report(&self) {
        let elapsed = self.total_start.elapsed();
        if let Some(path) = &self.json_path {
            let phases: Vec<_> = self.phases.iter().map(|(name, duration)| {
                serde_json::json!({"name": name, "seconds": duration.as_secs_f64()})
            }).collect();
            let report = serde_json::json!({
                "schema_version": 1,
                "total_seconds": elapsed.as_secs_f64(),
                "object_cache_hit": self.object_cache_hit,
                "phases": phases,
            });
            if let Err(error) = std::fs::write(path, report.to_string()) {
                eprintln!("could not write timings to {}: {error}", path.display());
            }
        }
        if !self.enabled {
            return;
        }

        crate::build_output::timings(&self.phases, elapsed, self.object_cache_hit);
    }

    fn record(&mut self, name: &'static str, duration: std::time::Duration) {
        if self.enabled || self.json_path.is_some() {
            self.phases.push((name, duration));
        }
    }
}

impl Drop for CompilationTimings {
    fn drop(&mut self) {
        self.report();
    }
}

/// Analogous to the `ginc` command
pub struct GinCompiler;

/// Final compiler outcome.
#[derive(Debug)]
pub enum CompileResult {
    /// Compilation succeeded.
    Success { output: Option<PathBuf> },
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
    #[error("type checking failed")]
    TypecheckFailed,
    /// Compiler input violated a structural driver invariant.
    #[error("compiler driver failed: {0}")]
    Driver(CompilerDriverError),
    /// Target resolution failed.
    #[error("{0}")]
    InvalidTarget(flask::TargetError),
    /// Entry package did not define a concrete target.
    #[error("entry package must set `target` to a full target triple")]
    MissingEntryTarget,
    #[error("artifact storage failed: {0}")]
    ArtifactStorage(std::io::Error),
    #[error("{0}")]
    Resolution(flask::ResolutionError),
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
    pub fn compile_cli(args: &mut Args) -> CompileResult {
        let output = crate::build_output::BuildOutput::new(args);
        let result = Self::compile(args);
        output.finish(&result, args);
        result
    }

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
        let mut reporter = DiagnosticReporter::new(&args.input, CheckOptions::default());
        Self::run(args, false, &mut reporter)
    }

    pub fn check(args: &mut Args, options: CheckOptions) -> CompileResult {
        let mut reporter = DiagnosticReporter::new(&args.input, options);
        let result = Self::run(args, true, &mut reporter);
        if let CompileResult::Failed(error) = &result
            && !matches!(
                error,
                CompileFailure::ParseFailed | CompileFailure::TypecheckFailed
            )
        {
            reporter.failure(&error.to_string());
        }
        reporter.finish(result.is_success());
        result
    }

    fn run(args: &mut Args, check_only: bool, reporter: &mut DiagnosticReporter) -> CompileResult {
        let path = args.input.to_owned();
        let mut timings = CompilationTimings::new(
            args.timings || CompilationTimings::enabled(),
            args.timings_json.clone(),
        );

        let file_paths = timings.run("discover-sources", || path.collect_gin_files());
        if file_paths.is_empty() {
            return CompileResult::Failed(CompileFailure::NoInputFiles { path });
        }

        let is_library = path.is_dir();
        let sources = match timings.run("read-sources", || read_sources(&file_paths)) {
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
        if print_parse_diagnostics(&parsed, reporter) {
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
        let flask_config = match FlaskConfig::discover(&entry_dir) {
            Ok(config) => config.map(|(config, _)| config),
            Err(error) => {
                return CompileResult::Failed(CompileFailure::Resolution(flask::ResolutionError(
                    error,
                )));
            }
        };
        if let Some(config) = &flask_config
            && let Err(error) = config.validate_resolution()
        {
            return CompileResult::Failed(CompileFailure::Resolution(error));
        }
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
                    .with_gin_home(args.gin_home.clone())
                    .with_dependencies(args.dependencies.clone())
                    .with_import_resolution(use_package_resolver),
            )
        });

        let has_front_end_flaws = print_diagnostics(&checked, reporter);
        if has_front_end_flaws && !check_only {
            return CompileResult::Failed(CompileFailure::ParseFailed);
        }

        let has_typecheck_flaws = timings.run("typecheck-diagnostics", || {
            print_type_diagnostics(&checked, reporter)
        });
        if has_front_end_flaws {
            return CompileResult::Failed(CompileFailure::ParseFailed);
        }
        if has_typecheck_flaws {
            return CompileResult::Failed(CompileFailure::TypecheckFailed);
        }

        if check_only {
            return CompileResult::Success { output: None };
        }

        let emit_result = match args.emit {
            crate::cli::Emit::Mlir => emit_mlir_typed(&driver, &checked, &mut timings),
            crate::cli::Emit::Interface => emit_interface(&checked, args),
            crate::cli::Emit::Obj | crate::cli::Emit::Exe => emit_native_typed(
                &driver,
                &checked,
                NativeEmitOptions {
                    args,
                    fast_llvm,
                    timings: &mut timings,
                },
            ),
        };

        let output = match emit_result {
            Ok(output) => output,
            Err(failure) => return CompileResult::Failed(failure),
        };

        CompileResult::Success { output }
    }

    pub(crate) fn fast_llvm_enabled() -> bool {
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
    pub fn has_typecheck_diagnostics(&self) -> bool {
        matches!(self, Self::Failed(CompileFailure::TypecheckFailed))
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
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
fn print_parse_diagnostics(package: &ParsedPackage, reporter: &mut DiagnosticReporter) -> bool {
    let mut has_flaws = false;
    for file in package.files() {
        has_flaws |= reporter.emit(&file.path, &file.source, &file.output.symptoms);
    }
    has_flaws
}

fn print_diagnostics(package: &CheckedPackage, reporter: &mut DiagnosticReporter) -> bool {
    let mut has_flaws = false;
    for file in package.files() {
        has_flaws |= reporter.emit(file.path(), file.source(), file.front_end_diagnostics());
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
fn print_type_diagnostics(package: &CheckedPackage, reporter: &mut DiagnosticReporter) -> bool {
    let mut has_flaws = false;
    for file in package.files() {
        has_flaws |= reporter.emit(file.path(), file.source(), file.typecheck_diagnostics());
        reporter.emit(file.path(), file.source(), file.typed().all_warnings());
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
    timings: &mut CompilationTimings,
) -> Result<Option<PathBuf>, CompileFailure> {
    let context = codegen::emit::NativeCompiler::create_context();
    let lowered = timings.run("mlir-lowering", || driver.lower_package(&context, package));
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
    fast_llvm: bool,
    timings: &'a mut CompilationTimings,
}

fn emit_native_typed(
    driver: &CompilerDriver,
    package: &CheckedPackage,
    options: NativeEmitOptions<'_>,
) -> Result<Option<PathBuf>, CompileFailure> {
    let NativeEmitOptions {
        args,
        fast_llvm,
        timings,
    } = options;
    let dir = timings.run("artifact-key", || artifact_directory(package, args))?;
    let staging = tempfile::tempdir_in(&dir).map_err(CompileFailure::ArtifactStorage)?;
    let obj_path = staging.path().join("module.o");

    let identity = if args.no_cache {
        None
    } else {
        timings.run(
            "toolchain-identity",
            codegen::emit::NativeCompiler::object_cache_identity,
        )
    };
    let cached_object = dir.join("module.o");
    let receipt = dir.join("module.o.sha256");
    let lookup = || {
        identity.as_ref().is_some_and(|identity| {
            verified_object(&cached_object, &receipt, identity, &obj_path).unwrap_or(false)
        })
    };
    let reused = timings.run("object-cache-lookup", lookup);
    timings.object_cache_hit = Some(reused);
    if !reused {
        let profile = args.profile;

        let compiler = codegen::emit::NativeCompiler::default();
        let context = codegen::emit::NativeCompiler::create_context();
        let lowered = timings.run("mlir-lowering", || driver.lower_package(&context, package));
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
        let (ok, more, native_timings) = compiler.native_from_module_with_timings_for_target(
            &module,
            &obj_path,
            profile,
            fast_llvm,
            package.target().triple_raw(),
        );
        timings.record("mlir-optimization", native_timings.optimization);
        timings.record("llvm-lowering", native_timings.lowering);
        timings.record("object-emission", native_timings.object_emission);
        if !ok {
            print_codegen_diagnostics(package.primary_file(), &more);
            symptoms.extend(more);
            return Err(CompileFailure::EmissionFailed {
                stage: CompileEmitStage::ObjectEmission,
                diagnostics: symptoms,
            });
        }

        if symptoms.is_empty()
            && more.is_empty()
            && let Some(identity) = &identity
        {
            cache_object(&obj_path, &cached_object, &receipt, identity)
                .map_err(CompileFailure::ArtifactStorage)?;
        }
    }

    if matches!(args.emit, crate::cli::Emit::Exe) {
        let exe_path = staging.path().join("module");
        let (linked, link_symptoms) = timings.run("linking", || {
            codegen::emit::NativeCompiler::link_executable(
                &obj_path,
                &exe_path,
                package.target().triple_raw(),
            )
        });
        if !linked {
            print_codegen_diagnostics(package.primary_file(), &link_symptoms);
            return Err(CompileFailure::EmissionFailed {
                stage: CompileEmitStage::Linking,
                diagnostics: link_symptoms,
            });
        }
        publish_output(&exe_path, &dir.join("module"), args)
    } else {
        publish_output(&obj_path, &dir.join("module.o"), args)
    }
}

fn object_digest(bytes: &[u8], identity: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"gin.object-cache.v1\0");
    hash.update(identity);
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

fn verified_object(
    cached: &Path,
    receipt: &Path,
    identity: &[u8],
    staged: &Path,
) -> std::io::Result<bool> {
    let bytes = std::fs::read(cached)?;
    if std::fs::read_to_string(receipt)? != object_digest(&bytes, identity) {
        return Ok(false);
    }
    std::fs::write(staged, bytes)?;
    Ok(true)
}

fn cache_object(
    staged: &Path,
    cached: &Path,
    receipt: &Path,
    identity: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    let bytes = std::fs::read(staged)?;
    let parent = cached
        .parent()
        .ok_or_else(|| std::io::Error::other("object has no cache directory"))?;
    let mut object = tempfile::NamedTempFile::new_in(parent)?;
    object.write_all(&bytes)?;
    let mut record = tempfile::NamedTempFile::new_in(parent)?;
    record.write_all(object_digest(&bytes, identity).as_bytes())?;
    object.persist(cached).map_err(|e| e.error)?;
    record.persist(receipt).map_err(|e| e.error)?;
    Ok(())
}

fn emit_interface(
    package: &CheckedPackage,
    args: &Args,
) -> Result<Option<PathBuf>, CompileFailure> {
    if package.files().iter().any(|file| {
        file.typecheck_diagnostics()
            .iter()
            .any(|d| d.category == Category::Flaw)
    }) {
        return Err(CompileFailure::EmissionFailed {
            stage: CompileEmitStage::InterfaceEmission,
            // interface-invalid-package
            diagnostics: vec![Diagnostic::new(
                InterfaceCode::InvalidPackage,
                "cannot emit an authoritative interface for a package with type errors",
            )],
        });
    }
    let dir = artifact_directory(package, args)?;
    let staging = tempfile::tempdir_in(&dir).map_err(CompileFailure::ArtifactStorage)?;
    let path = staging.path().join("module.ginif");
    std::fs::write(&path, package.public_interface().canonical_bytes())
        .map_err(CompileFailure::ArtifactStorage)?;
    publish_output(&path, &dir.join("module.ginif"), args)
}

fn artifact_directory(package: &CheckedPackage, args: &Args) -> Result<PathBuf, CompileFailure> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    static COMPILER: std::sync::OnceLock<Result<Vec<u8>, String>> = std::sync::OnceLock::new();
    let compiler = COMPILER
        .get_or_init(|| {
            use std::io::Read;
            let mut file = std::fs::File::open(std::env::current_exe().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
            }
            Ok(hash.finalize().to_vec())
        })
        .as_ref()
        .map_err(|e| CompileFailure::ArtifactStorage(std::io::Error::other(e.clone())))?;
    hash.update(compiler);
    hash.update(package.public_interface().canonical_bytes());
    let mut files: Vec<_> = package.files().iter().collect();
    files.sort_by_key(|file| file.path());
    let mut manifests = std::collections::BTreeSet::new();
    for file in files {
        if let Some((_, root)) = FlaskConfig::find_package_config(file.path()) {
            manifests.insert(FlaskConfig::manifest_path(&root));
        }
        for bytes in [
            file.path().as_os_str().as_encoded_bytes(),
            file.source().as_bytes(),
        ] {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
    for manifest in manifests {
        let bytes = std::fs::read(manifest).map_err(CompileFailure::ArtifactStorage)?;
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    hash.update([u8::from(args.fast_llvm || GinCompiler::fast_llvm_enabled())]);
    let home = args
        .gin_home
        .clone()
        .map(Ok)
        .unwrap_or_else(flask::GinHome::discover)
        .map_err(CompileFailure::ArtifactStorage)?;
    let dir = home
        .artifacts(
            &args.input,
            &hash.finalize(),
            args.profile.as_str(),
            package.target().triple_raw().unwrap_or("library"),
        )
        .map_err(CompileFailure::ArtifactStorage)?;
    std::fs::create_dir_all(&dir).map_err(CompileFailure::ArtifactStorage)?;
    Ok(dir)
}

fn publish_output(
    staged: &Path,
    cached: &Path,
    args: &Args,
) -> Result<Option<PathBuf>, CompileFailure> {
    std::fs::rename(staged, cached).map_err(CompileFailure::ArtifactStorage)?;
    let output = if let Some(export) = &args.output {
        if export != cached {
            let parent = export
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            std::fs::create_dir_all(parent).map_err(CompileFailure::ArtifactStorage)?;
            let tmp =
                tempfile::NamedTempFile::new_in(parent).map_err(CompileFailure::ArtifactStorage)?;
            std::fs::copy(cached, tmp.path()).map_err(CompileFailure::ArtifactStorage)?;
            tmp.persist(export)
                .map_err(|e| CompileFailure::ArtifactStorage(e.error))?;
        }
        export.clone()
    } else {
        cached.to_path_buf()
    };
    Ok(Some(output))
}
