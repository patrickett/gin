//! Source loading, checking, diagnostics and emission for one compiler invocation.

use crate::build_output::CompilationTimings;
use crate::cli::{Args, Emit, Profile};
use crate::compile::{CompileFailure, CompileResult};
use crate::reporting::{CheckOptions, DiagnosticReporter};
use diagnostic::code::SourceCheckCode;
use typecheck::{SourceCheckError, SourceCheckFailure, SourceCheckNote, SourceCheckNoteKind};

/// The failure with the reasons emission gave, when it gave any: a build that fails should say
/// why, not only in which stage.
fn failure_text(error: &CompileFailure) -> String {
    match error {
        CompileFailure::EmissionFailed { diagnostics, .. } if !diagnostics.is_empty() => {
            let reasons: Vec<_> = diagnostics.iter().map(|d| d.message.as_str()).collect();
            format!("{error}: {}", reasons.join("; "))
        }
        _ => error.to_string(),
    }
}

/// The targets `#target` can answer for. The first is not special: the default is the host.
const SUPPORTED_TARGETS: [typecheck::Target; 4] = [
    typecheck::Target::AARCH64_DARWIN,
    typecheck::Target::X86_64_DARWIN,
    typecheck::Target::AARCH64_LINUX,
    typecheck::Target::X86_64_LINUX,
];

fn parse_target(triple: &str) -> Result<typecheck::Target, String> {
    SUPPORTED_TARGETS
        .into_iter()
        .find(|target| target.triple == triple)
        .ok_or_else(|| {
            let supported: Vec<_> = SUPPORTED_TARGETS.iter().map(|t| t.triple).collect();
            format!(
                "unknown target `{triple}`; supported targets: {}",
                supported.join(", ")
            )
        })
}

/// What `#target` answers: `--target`, else the package's `mod.flask` `target:` when it names a
/// machine (`library` or no key means the host), else the host.
#[cfg(test)]
pub(crate) fn resolve_target(args: &Args) -> Result<typecheck::Target, String> {
    resolve_target_in(args, args.input.is_dir().then_some(args.input.as_path()))
}

/// As [`resolve_target`], reading the manifest of the package at `root`.
fn resolve_target_in(
    args: &Args,
    root: Option<&std::path::Path>,
) -> Result<typecheck::Target, String> {
    if let Some(triple) = &args.target {
        return parse_target(triple);
    }
    if let Some(root) = root
        && let Ok(raw) = std::fs::read_to_string(flask::FlaskConfig::manifest_path(root))
        && let Ok(config) = flask::FlaskConfig::parse_manifest(&raw)
        && let Some(triple) = config.target()
        && triple != "library"
    {
        return parse_target(triple);
    }
    Ok(typecheck::Target::host())
}

/// What an input names: a package (and the module of it that is the root of this run), or a
/// `.gin` file that lies in no package.
#[derive(Debug, PartialEq, Eq)]
enum Input {
    File(std::path::PathBuf),
    Package {
        root: std::path::PathBuf,
        module: Vec<String>,
    },
}

/// The package that holds `directory`, with the folder path from its root to `directory`.
fn enclosing_package(directory: &std::path::Path) -> Option<Input> {
    let directory = std::fs::canonicalize(directory).ok()?;
    let (_, root) = flask::FlaskConfig::find_package_config(&directory)?;
    let module = directory
        .strip_prefix(&root)
        .ok()?
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(Input::Package { root, module })
}

/// A directory with a `mod.flask` is a package whose root module is its own `.gin` files. A
/// folder inside a package, or a file inside one, selects that package with the folder as the
/// root module. Only a `.gin` file outside every package is checked alone.
fn resolve_input(args: &Args) -> Result<Input, String> {
    let input = &args.input;
    if input.is_dir() {
        if flask::FlaskConfig::manifest_path(input).is_file() {
            return Ok(Input::Package {
                root: input.clone(),
                module: Vec::new(),
            });
        }
        // Without a package around it, `Package::open` says the manifest is missing.
        return Ok(enclosing_package(input).unwrap_or(Input::Package {
            root: input.clone(),
            module: Vec::new(),
        }));
    }
    if input.extension().is_none_or(|extension| extension != "gin") || !input.is_file() {
        return Err("expected a package directory or an existing .gin file".to_owned());
    }
    let parent = input
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    Ok(enclosing_package(parent).unwrap_or_else(|| Input::File(input.clone())))
}

/// Check, then perform the selected emission. Diagnostics, the check summary,
/// and a failure that has no diagnostic of its own are reported here.
pub(crate) fn run(args: &Args, options: CheckOptions, mode: Emit) -> CompileResult {
    let mut reporter = DiagnosticReporter::new(options);
    let mut timings = CompilationTimings::new(
        args.timings || CompilationTimings::enabled(),
        args.timings_json.clone(),
    );
    let result = run_mode(args, mode, &mut reporter, &mut timings);
    if let CompileResult::Failed(error) = &result
        && !matches!(
            error,
            CompileFailure::ParseFailed | CompileFailure::TypecheckFailed
        )
    {
        reporter.failure(&failure_text(error));
    }
    reporter.finish(result.is_success());
    drop(timings);
    result
}

fn run_mode(
    args: &Args,
    mode: Emit,
    reporter: &mut DiagnosticReporter,
    timings: &mut CompilationTimings,
) -> CompileResult {
    let invalid = CompileFailure::InvalidOptions;
    if let Err(message) = validate_options(args, mode) {
        return CompileResult::Failed(invalid(message));
    }
    let input = match resolve_input(args) {
        Ok(input) => input,
        Err(message) => return CompileResult::Failed(invalid(message)),
    };
    match input {
        Input::File(path) => run_file(args, mode, &path, reporter, timings),
        Input::Package { root, module } => {
            run_package(args, mode, &root, &module, reporter, timings)
        }
    }
}

fn run_file(
    args: &Args,
    mode: Emit,
    path: &std::path::Path,
    reporter: &mut DiagnosticReporter,
    timings: &mut CompilationTimings,
) -> CompileResult {
    let invalid = CompileFailure::InvalidOptions;
    let mode = if mode == Emit::Auto {
        Emit::Check
    } else {
        mode
    };
    if let Err(message) = validate_options(args, mode) {
        return CompileResult::Failed(invalid(message));
    }
    if matches!(mode, Emit::Exe | Emit::Obj | Emit::Interface) {
        let message = "a file outside any package cannot be built; put it in a package (a directory with a mod.flask)";
        return CompileResult::Failed(invalid(message.to_owned()));
    }
    let checked = match timings.run("check", || load_and_check(args, path, reporter, invalid)) {
        Ok(checked) => checked,
        Err(error) => return CompileResult::Failed(error),
    };
    if mode != Emit::Mlir {
        return CompileResult::Success { output: None };
    }
    let lowered = timings.run("lower", || crate::lower_next::lower_file(&checked));
    match lowered {
        Ok(text) => write_mlir(args, &[(String::new(), text)]),
        Err(error) => CompileResult::Failed(CompileFailure::Lowering(error.to_string())),
    }
}

/// MLIR goes to `--output` when given. Otherwise it goes to stdout, one fenced block per
/// module; `validate_options` has refused that when stdout carries JSON diagnostics.
fn write_mlir(args: &Args, modules: &[(String, String)]) -> CompileResult {
    let Some(output) = &args.output else {
        for (name, text) in modules {
            if name.is_empty() {
                println!("\n```mlir\n{text}```\n");
            } else {
                println!("\n```mlir\n// gin module: {name}\n{text}```\n");
            }
        }
        return CompileResult::Success { output: None };
    };
    let mut joined = String::new();
    for (name, text) in modules {
        if !name.is_empty() {
            joined.push_str(&format!("// gin module: {name}\n"));
        }
        joined.push_str(text);
    }
    match std::fs::write(output, joined) {
        Ok(()) => CompileResult::Success {
            output: Some(output.clone()),
        },
        Err(error) => CompileResult::Failed(CompileFailure::ArtifactStorage(error)),
    }
}

fn run_package(
    args: &Args,
    mode: Emit,
    root: &std::path::Path,
    module: &[String],
    reporter: &mut DiagnosticReporter,
    timings: &mut CompilationTimings,
) -> CompileResult {
    let invalid = CompileFailure::InvalidOptions;
    let checked = match load_and_check_package(args, root, module, reporter, timings, invalid) {
        Ok(checked) => checked,
        Err(error) => return CompileResult::Failed(error),
    };
    assert_eq!(checked.graph.modules.len(), checked.capsules.len());
    let mode = match mode {
        Emit::Auto if checked.entry().is_some() => Emit::Exe,
        Emit::Auto => Emit::Check,
        other => other,
    };
    if let Err(message) = validate_options(args, mode) {
        return CompileResult::Failed(invalid(message));
    }
    let built = match mode {
        Emit::Check | Emit::Auto => return CompileResult::Success { output: None },
        Emit::Interface => {
            // TODO: define the replacement bundle format and producer/provenance checks before
            // enabling publication; the legacy interface format is intentionally not emitted.
            return CompileResult::Failed(CompileFailure::InvalidOptions(
                "package interface bundle emission is not implemented; the legacy interface format is no longer emitted".to_owned(),
            ));
        }
        Emit::Mlir => {
            return match timings.run("lower", || crate::build_new::lower_package(&checked)) {
                Ok(blocks) => write_mlir(args, &blocks),
                Err(error) => CompileResult::Failed(error),
            };
        }
        Emit::Exe => build_executable(args, root, &checked, timings),
        Emit::Obj => build_objects(args, root, &checked, timings),
    };
    match built {
        Ok(output) => CompileResult::Success {
            output: Some(output),
        },
        Err(error) => CompileResult::Failed(error),
    }
}

/// `<package>/target/<profile>/<name>`, `name` being the last part of the root module's name.
fn default_output(
    root: &std::path::Path,
    args: &Args,
    checked: &package::CheckedPackage,
) -> std::path::PathBuf {
    let name = checked
        .graph
        .root
        .map(|index| checked.graph.modules[index].name.as_str())
        .unwrap_or("library");
    let name = name.rsplit('.').next().unwrap_or(name);
    root.join("target").join(args.profile.as_str()).join(name)
}

fn build_options(
    args: &Args,
    root: &std::path::Path,
) -> Result<crate::build_new::BuildOptions<'static>, CompileFailure> {
    let target = resolve_target_in(args, Some(root)).map_err(CompileFailure::InvalidOptions)?;
    // Only a foreign target is named to the object compiler and the linker; the host is their default.
    let foreign = (target != typecheck::Target::host()).then_some(target.triple);
    Ok(crate::build_new::BuildOptions {
        profile: args.profile,
        fast_llvm: args.fast_llvm || crate::compile::GinCompiler::fast_llvm_enabled(),
        target: foreign,
    })
}

fn build_executable(
    args: &Args,
    root: &std::path::Path,
    checked: &package::CheckedPackage,
    timings: &mut CompilationTimings,
) -> Result<std::path::PathBuf, CompileFailure> {
    let output = match &args.output {
        Some(output) => output.clone(),
        None => {
            let output = default_output(root, args, checked);
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(CompileFailure::ArtifactStorage)?;
            }
            output
        }
    };
    let options = build_options(args, root)?;
    crate::build_new::build_executable(checked, &output, options, timings)?;
    Ok(output)
}

fn build_objects(
    args: &Args,
    root: &std::path::Path,
    checked: &package::CheckedPackage,
    timings: &mut CompilationTimings,
) -> Result<std::path::PathBuf, CompileFailure> {
    let directory = match &args.output {
        Some(output) => output.clone(),
        None => {
            let mut directory = default_output(root, args, checked).into_os_string();
            directory.push(".objects");
            directory.into()
        }
    };
    let options = build_options(args, root)?;
    crate::build_new::build_objects(checked, &directory, options, timings)?;
    Ok(directory)
}

fn load_and_check(
    args: &Args,
    input: &std::path::Path,
    reporter: &mut DiagnosticReporter,
    invalid: fn(String) -> CompileFailure,
) -> Result<typecheck::CheckedSourceFile, CompileFailure> {
    let source =
        std::fs::read_to_string(input).map_err(|error| CompileFailure::UnreadableSource {
            path: input.to_owned(),
            error: error.to_string(),
        })?;
    let file = match parser::parse_strict(&source) {
        Ok(file) => file,
        Err(diagnostics) => {
            reporter.emit(input, &source, &diagnostics);
            return Err(CompileFailure::ParseFailed);
        }
    };
    let target = resolve_target_in(args, None).map_err(invalid)?;
    let limits = typecheck::Limits {
        target,
        // Migration: flip to `Wrapped` when `modules/core` has no bare type member left in an `or` list.
        ..typecheck::Limits::default()
    };
    match typecheck::check_file_diagnostic(&file, limits) {
        Ok(checked) => {
            report_notes(reporter, input, &source, &checked.notes);
            Ok(checked)
        }
        Err(failure) => {
            report_failure(reporter, input, &source, failure);
            Err(CompileFailure::TypecheckFailed)
        }
    }
}

fn validate_options(args: &Args, mode: Emit) -> Result<(), String> {
    // What only a build has a use for.
    let builds = matches!(mode, Emit::Exe | Emit::Obj | Emit::Auto);
    let writes = builds || matches!(mode, Emit::Mlir | Emit::Interface);
    let unsupported = [
        (args.output.is_some() && !writes, "--output"),
        (args.profile != Profile::Debug && !builds, "--profile"),
        (args.no_cache && !builds, "--no-cache"),
        (args.fast_llvm && !builds, "--fast-llvm"),
    ];
    for (present, name) in unsupported {
        if present {
            return Err(format!("{name} is not supported"));
        }
    }
    // Stdout carries the JSON stream, so it cannot also carry MLIR text.
    if mode == Emit::Mlir
        && args.message_format == crate::reporting::MessageFormat::Json
        && args.output.is_none()
    {
        return Err("--message-format json needs --output when emitting MLIR".to_owned());
    }
    Ok(())
}

fn load_and_check_package(
    args: &Args,
    root: &std::path::Path,
    module: &[String],
    reporter: &mut DiagnosticReporter,
    timings: &mut CompilationTimings,
    invalid: fn(String) -> CompileFailure,
) -> Result<package::CheckedPackage, CompileFailure> {
    let target = resolve_target_in(args, Some(root)).map_err(invalid)?;
    let limits = package::GraphLimits::default();
    // A library module is many declarations in one file (`core.primitive` provides a dozen
    // traits to one domain), so a package gets ten times the work a single file does.
    let work = typecheck::Limits {
        work_max: typecheck::Limits::default().work_max * 160,
        // Every import copies the whole function table of its dependency, which already holds the
        // tables of its own: a client of `core.io` and `core.string` holds ~4400 entries, over the
        // 4096 a single file is given. A package is allowed what its imports bring.
        definitions_max: 65536,
        target,
        // Migration: flip to `Wrapped` when `modules/core` has no bare type member left in an `or` list.
        ..typecheck::Limits::default()
    };
    // A library with nothing at its root is checked as every module it holds.
    let loaded = timings.run("load", || {
        let package = package::Package::open(root)?;
        let graph = if module.is_empty() && !package.has_sources_at(module) {
            package.load_graph_all(limits)
        } else {
            package.load_graph_at(module, limits)
        }?;
        Ok::<_, package::GraphError>((package, graph))
    });
    let (package, graph) = match loaded {
        Ok(loaded) => loaded,
        Err(package::GraphError::Load(package::LoadError::Syntax {
            path,
            diagnostics,
        })) => {
            let source = read_source(&path)?;
            reporter.set_dependency(dependency_source(root, &path));
            reporter.emit(&path, &source, &diagnostics);
            reporter.set_dependency(false);
            return Err(CompileFailure::ParseFailed);
        }
        Err(error) => return Err(CompileFailure::Package(error.to_string())),
    };
    let checked = timings.run("check", || {
        package.check_graph(graph, work).map_err(Box::new)
    });
    match checked {
        Ok(checked) => {
            report_package_notes(reporter, &checked, package.name(), args.warnings);
            Ok(checked)
        }
        Err(error) => match *error {
            package::PackageError::Check {
                module,
                path,
                failure,
            } => {
                reporter.set_dependency(module.split('.').next() != Some(package.name()));
                let failure = report_package_check(reporter, root, &module, path, failure);
                reporter.set_dependency(false);
                Err(failure)
            }
            other => Err(CompileFailure::Package(other.to_string())),
        },
    }
}

fn dependency_source(root: &std::path::Path, source: &std::path::Path) -> bool {
    let directory = source.parent().unwrap_or(root);
    let Some(Input::Package { root: origin, .. }) = enclosing_package(directory) else {
        return false;
    };
    std::fs::canonicalize(root).is_ok_and(|root| root != origin)
}

fn report_package_notes(
    reporter: &mut DiagnosticReporter,
    checked: &package::CheckedPackage,
    own_package: &str,
    scope: crate::reporting::WarningScope,
) {
    for (index, capsule) in checked.capsules.iter().enumerate() {
        let module = &checked.graph.modules[index];
        let own = module.name.split('.').next() == Some(own_package);
        for note in &capsule.checked().notes {
            if !own && scope == crate::reporting::WarningScope::Own {
                if diagnostic::code::Code::category(&note_code(note.kind))
                    == diagnostic::Category::Warn
                {
                    reporter.hide_warning();
                }
                continue;
            }
            let path = note
                .file
                .and_then(|file| module.paths.get(file))
                .map(std::path::PathBuf::as_path);
            reporter.set_dependency(!own);
            match path.and_then(|path| Some((path, read_source(path).ok()?))) {
                Some((path, source)) => {
                    report_notes(reporter, path, &source, std::slice::from_ref(note));
                }
                None => {
                    let root = module
                        .paths
                        .first()
                        .map_or(std::path::Path::new("."), |p| p);
                    reporter.emit_unlocated(root, note_code(note.kind).into(), &note.detail);
                }
            }
            reporter.set_dependency(false);
        }
    }
}

fn report_package_check(
    reporter: &mut DiagnosticReporter,
    root: &std::path::Path,
    module: &str,
    path: Option<std::path::PathBuf>,
    failure: SourceCheckFailure,
) -> CompileFailure {
    match path {
        Some(path) => {
            let Ok(source) = read_source(&path) else {
                return CompileFailure::UnreadableSource {
                    path,
                    error: "source changed while checking".to_owned(),
                };
            };
            report_failure(reporter, &path, &source, failure);
        }
        None => {
            let (code, message) = failure_details(failure.error);
            let message = failure.detail.clone().unwrap_or_else(|| message.to_owned());
            // No file to point at: say which module failed.
            let message = format!("in module `{module}`: {message}");
            reporter.emit_unlocated(root, code.into(), &message);
        }
    }
    CompileFailure::TypecheckFailed
}

fn read_source(path: &std::path::Path) -> Result<String, CompileFailure> {
    std::fs::read_to_string(path).map_err(|error| CompileFailure::UnreadableSource {
        path: path.to_owned(),
        error: error.to_string(),
    })
}

/// The code a note is reported under; its category (a warning) comes from the code.
pub fn note_code(kind: SourceCheckNoteKind) -> SourceCheckCode {
    match kind {
        SourceCheckNoteKind::RedundantTerminal => SourceCheckCode::RedundantTerminal,
        SourceCheckNoteKind::UnreachableCode => SourceCheckCode::UnreachableCode,
        SourceCheckNoteKind::Hole => SourceCheckCode::Hole,
    }
}

fn report_notes(
    reporter: &mut DiagnosticReporter,
    path: &std::path::Path,
    source: &str,
    notes: &[SourceCheckNote],
) {
    for note in notes {
        let code = note_code(note.kind);
        match note.span {
            Some(_) => {
                reporter.emit(path, source, &[crate::flaw::note_diagnostic(note)]);
            }
            None => reporter.emit_unlocated(path, code.into(), &note.detail),
        }
    }
}

fn report_failure(
    reporter: &mut DiagnosticReporter,
    path: &std::path::Path,
    source: &str,
    failure: SourceCheckFailure,
) {
    // The one definition shared with the REPL and the JSONL protocol: `crate::flaw`.
    if failure.span.is_some() {
        reporter.emit(path, source, &[crate::flaw::failure_diagnostic(&failure)]);
    } else {
        let (code, _) = failure_details(failure.error);
        reporter.emit_unlocated(path, code.into(), &crate::flaw::failure_message(&failure));
    }
}

pub fn failure_details(error: SourceCheckError) -> (SourceCheckCode, &'static str) {
    match error {
        SourceCheckError::Budget => (
            SourceCheckCode::Budget,
            "source checker work budget exceeded",
        ),
        SourceCheckError::Malformed => (SourceCheckCode::Malformed, "malformed source construct"),
        SourceCheckError::Unsupported => (
            SourceCheckCode::Unsupported,
            "construct is not supported by the source checker",
        ),
        SourceCheckError::Duplicate => (
            SourceCheckCode::Duplicate,
            "source checker reported a duplicate",
        ),
        SourceCheckError::UnknownName => (SourceCheckCode::UnknownName, "unknown name"),
        SourceCheckError::MissingBody => (SourceCheckCode::MissingBody, "missing body"),
        SourceCheckError::Type => (SourceCheckCode::Type, "source type check failed"),
        SourceCheckError::Unproved => (
            SourceCheckCode::Unproved,
            "source proof obligation not proved",
        ),
        SourceCheckError::Overflow => (
            SourceCheckCode::Overflow,
            "source checker reported overflow",
        ),
        SourceCheckError::UseAfterMove => (SourceCheckCode::UseAfterMove, "use after move"),
        SourceCheckError::Unconsumed => (SourceCheckCode::Unconsumed, "unconsumed value"),
        SourceCheckError::Privacy => (SourceCheckCode::Privacy, "source privacy check failed"),
        SourceCheckError::InstructionTarget => (
            SourceCheckCode::InstructionTarget,
            "`#instruction` marks a body-less function, whose unannotated parameter takes a literal plain name",
        ),
        SourceCheckError::WrapTarget => (
            SourceCheckCode::WrapTarget,
            "`#wrap` applies only to a named integer domain",
        ),
        SourceCheckError::UnrestrictedTarget => (
            SourceCheckCode::UnrestrictedTarget,
            "`#unrestricted` applies only to a non-generic integer domain or record",
        ),
        SourceCheckError::UnrestrictedField => (
            SourceCheckCode::UnrestrictedField,
            "an `#unrestricted` record may only contain unrestricted fields",
        ),
        SourceCheckError::MethodReceiver => (
            SourceCheckCode::MethodReceiver,
            "a method must take `self` as its first parameter",
        ),
        SourceCheckError::MissingTerminal => (
            SourceCheckCode::MissingTerminal,
            "a linear domain needs a method that takes `self` and ends it",
        ),
        SourceCheckError::InfixOperator => (
            SourceCheckCode::InfixOperator,
            "`#infix` needs a supported operator ('+', '-' or '*') on a method taking `self` and one operand",
        ),
        SourceCheckError::MoveOutOfBorrow => (
            SourceCheckCode::MoveOutOfBorrow,
            "a `ref` or `mut` borrow cannot be moved, ended, or returned; only its owner can",
        ),
        SourceCheckError::EndEscape => (
            SourceCheckCode::EndEscape,
            "an `end` value must be ended in this function and cannot escape through a result",
        ),
        SourceCheckError::Immutable => (
            SourceCheckCode::Immutable,
            "only a `mut` parameter or a `:` local can be reassigned with `::`",
        ),
        SourceCheckError::OverwriteLinear => (
            SourceCheckCode::OverwriteLinear,
            "assigning here would drop a linear value; it has to be ended or moved first",
        ),
        SourceCheckError::BorrowEscape => (
            SourceCheckCode::BorrowEscape,
            "a borrow cannot outlive the value it borrows: it would be returned or stored beyond its source's scope",
        ),
        SourceCheckError::MissingAcceptance => (
            SourceCheckCode::MissingAcceptance,
            "this operation stops being checked; accept that by writing the operation inside `#unchecked(...)`",
        ),
        SourceCheckError::UnneededAcceptance => (
            SourceCheckCode::UnneededAcceptance,
            "`#unchecked(...)` surrounds no operation that skips a proof; remove it",
        ),
        SourceCheckError::WorldLimit => (
            SourceCheckCode::WorldLimit,
            "a function makes too many outcome combinations of its calls; split it or make the unused call's result explicit",
        ),
        SourceCheckError::BorrowConflict => (
            SourceCheckCode::BorrowConflict,
            "a value is borrowed and used in a way that conflicts within one call",
        ),
        SourceCheckError::UnrestrictedArgument => (
            SourceCheckCode::UnrestrictedArgument,
            "a linear type cannot be passed to an `#unrestricted` type, which may only hold unrestricted things",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::GinCompiler;
    use crate::reporting::MessageFormat;

    fn args(input: std::path::PathBuf) -> Args {
        Args {
            input,
            emit: Emit::Check,
            ..Args::default()
        }
    }

    #[test]
    fn an_input_is_a_package_a_folder_in_one_or_a_file_outside_any() {
        let directory = tempfile::tempdir().unwrap();
        let package = directory.path().join("pkg");
        std::fs::create_dir_all(package.join("inner")).unwrap();
        std::fs::write(
            package.join("mod.flask"),
            "#settings\nmodule: \"pkg\"\nversion: 0.1.0\ntarget: \"library\"\n",
        )
        .unwrap();
        std::fs::write(package.join("inner/a.gin"), "").unwrap();
        std::fs::write(package.join("root.gin"), "").unwrap();
        let root = std::fs::canonicalize(&package).unwrap();
        // The package directory itself is kept as given.
        assert_eq!(
            resolve_input(&args(package.clone())).unwrap(),
            Input::Package {
                root: package.clone(),
                module: vec![]
            }
        );
        // A folder or a file inside it selects the package and the folder as its module.
        for input in [package.join("inner"), package.join("inner/a.gin")] {
            assert_eq!(
                resolve_input(&args(input)).unwrap(),
                Input::Package {
                    root: root.clone(),
                    module: vec!["inner".into()]
                }
            );
        }
        assert_eq!(
            resolve_input(&args(package.join("root.gin"))).unwrap(),
            Input::Package {
                root: root.clone(),
                module: vec![]
            }
        );
        // Outside a package, a file stands alone and a directory has no manifest.
        let loose = directory.path().join("loose.gin");
        std::fs::write(&loose, "").unwrap();
        assert_eq!(
            resolve_input(&args(loose.clone())).unwrap(),
            Input::File(loose)
        );
        assert!(resolve_input(&args(directory.path().join("missing.gin"))).is_err());
        let other = directory.path().join("source.txt");
        std::fs::write(&other, "").unwrap();
        assert!(resolve_input(&args(other)).is_err());
    }

    #[test]
    fn options_a_mode_has_no_use_for_are_rejected_before_reading_source() {
        let only_for_builds = [
            Args {
                output: Some("out".into()),
                ..Args::default()
            },
            Args {
                profile: Profile::Release,
                ..Args::default()
            },
            Args {
                no_cache: true,
                ..Args::default()
            },
            Args {
                fast_llvm: true,
                ..Args::default()
            },
        ];
        for case in only_for_builds {
            let error = validate_options(&case, Emit::Check).unwrap_err();
            assert!(error.contains("is not supported"), "{error}");
            validate_options(&case, Emit::Exe).unwrap();
            validate_options(&case, Emit::Obj).unwrap();
            validate_options(&case, Emit::Auto).unwrap();
        }
        // MLIR may be written to `--output`, but takes no build option beyond that.
        validate_options(
            &Args {
                output: Some("out".into()),
                ..Args::default()
            },
            Emit::Mlir,
        )
        .unwrap();
        assert!(
            validate_options(
                &Args {
                    profile: Profile::Release,
                    ..Args::default()
                },
                Emit::Mlir
            )
            .is_err()
        );
        // Timings measure every mode.
        let timed = Args {
            timings: true,
            timings_json: Some("t.json".into()),
            ..Args::default()
        };
        validate_options(&timed, Emit::Check).unwrap();
    }

    #[test]
    fn json_diagnostics_and_mlir_never_share_stdout() {
        let json = Args {
            message_format: MessageFormat::Json,
            ..Args::default()
        };
        let error = validate_options(&json, Emit::Mlir).unwrap_err();
        assert!(error.contains("--output"), "{error}");
        let to_file = Args {
            output: Some("out.mlir".into()),
            ..json
        };
        validate_options(&to_file, Emit::Mlir).unwrap();
        let human = Args::default();
        validate_options(&human, Emit::Mlir).unwrap();
    }

    #[test]
    fn a_target_is_accepted_and_an_unknown_one_is_refused() {
        let known = Args {
            target: Some("x86_64-unknown-linux-gnu".into()),
            ..Args::default()
        };
        assert!(validate_options(&known, Emit::Check).is_ok());
        assert_eq!(
            resolve_target(&known).unwrap().triple,
            "x86_64-unknown-linux-gnu"
        );
        for bad in ["library", "invalid-target"] {
            let unknown = Args {
                target: Some(bad.into()),
                ..Args::default()
            };
            let error = resolve_target(&unknown).unwrap_err();
            assert!(
                error.contains("unknown target") && error.contains("aarch64-apple-darwin"),
                "{error}"
            );
        }
    }

    #[test]
    fn public_entry_points_honor_mode_and_reject_emission() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.gin");
        std::fs::write(&source, "identity(x in 0 to 10) in 0 to 10: return x\n").unwrap();
        let mut options = args(source);
        assert!(matches!(
            GinCompiler::compile(&mut options),
            CompileResult::Success { output: None }
        ));
        assert!(matches!(
            GinCompiler::compile_cli(&mut options),
            CompileResult::Success { output: None }
        ));
        assert!(matches!(
            GinCompiler::check(&mut options, CheckOptions::default()),
            CompileResult::Success { output: None }
        ));
        options.emit = Emit::Obj;
        for result in [
            GinCompiler::compile(&mut options),
            GinCompiler::compile_cli(&mut options),
        ] {
            assert!(matches!(
                result,
                CompileResult::Failed(CompileFailure::InvalidOptions(_))
            ));
        }
        assert!(GinCompiler::check(&mut options, CheckOptions::default()).is_success());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn parse_flaws_do_not_reach_source_checking() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.gin");
        std::fs::write(&source, "bad(\n").unwrap();
        assert!(matches!(
            run(
                &args(source),
                CheckOptions {
                    message_format: MessageFormat::Json,
                    max_errors: Some(0)
                },
                Emit::Check
            ),
            CompileResult::Failed(CompileFailure::ParseFailed)
        ));
    }

    #[test]
    fn all_source_failures_have_independent_diagnostic_codes() {
        let errors = [
            SourceCheckError::Budget,
            SourceCheckError::Malformed,
            SourceCheckError::Unsupported,
            SourceCheckError::Duplicate,
            SourceCheckError::UnknownName,
            SourceCheckError::MissingBody,
            SourceCheckError::Type,
            SourceCheckError::Unproved,
            SourceCheckError::Overflow,
            SourceCheckError::UseAfterMove,
            SourceCheckError::Unconsumed,
            SourceCheckError::Privacy,
            SourceCheckError::InstructionTarget,
            SourceCheckError::WrapTarget,
            SourceCheckError::UnrestrictedTarget,
            SourceCheckError::UnrestrictedField,
            SourceCheckError::MethodReceiver,
            SourceCheckError::MissingTerminal,
            SourceCheckError::UnrestrictedArgument,
            SourceCheckError::InfixOperator,
            SourceCheckError::MoveOutOfBorrow,
            SourceCheckError::EndEscape,
            SourceCheckError::BorrowConflict,
            SourceCheckError::Immutable,
            SourceCheckError::OverwriteLinear,
            SourceCheckError::BorrowEscape,
            SourceCheckError::MissingAcceptance,
            SourceCheckError::UnneededAcceptance,
            SourceCheckError::WorldLimit,
        ];
        let mut codes: std::collections::HashSet<_> = errors
            .into_iter()
            .map(|error| {
                let (code, message) = failure_details(error);
                assert!(!message.is_empty());
                let code = code.to_string();
                assert!(code.starts_with("source-check-"));
                code
            })
            .collect();
        // Notes are non-fatal, so their codes come from the notes, not from errors.
        for kind in [
            SourceCheckNoteKind::RedundantTerminal,
            SourceCheckNoteKind::UnreachableCode,
            SourceCheckNoteKind::Hole,
        ] {
            let code = note_code(kind);
            assert_ne!(
                diagnostic::code::Code::category(&code),
                diagnostic::Category::Flaw
            );
            assert!(
                codes.insert(code.to_string()),
                "a note shares an error's code"
            );
        }
        assert_eq!(codes.len(), SourceCheckCode::all().count());
    }
}
