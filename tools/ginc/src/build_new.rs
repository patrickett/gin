//! Native executables from a checked package: one object per Gin module, joined by the
//! system linker. Nothing here consults the legacy typed AST or its backend entry points.

use std::path::{Path, PathBuf};

use backend::{NativeCompiler, Profile};
use package::CheckedPackage;

use crate::build_output::CompilationTimings;
use crate::compile::{CompileEmitStage, CompileFailure};

/// What a native build is told beyond the package.
#[derive(Clone, Copy)]
pub(crate) struct BuildOptions<'a> {
    pub profile: Profile,
    /// No MLIR optimization and `-O0`, whatever the profile.
    pub fast_llvm: bool,
    /// A foreign target triple; the host is the object compiler's and linker's default.
    pub target: Option<&'a str>,
}

/// The MLIR text of every module, `(module name, text)` in checking order. The root module is
/// the program: all of it must lower. A library may hold functions the program never calls.
pub(crate) fn lower_package(
    checked: &CheckedPackage,
) -> Result<Vec<(String, String)>, CompileFailure> {
    let capsules = checked.capsules.iter().collect::<Vec<_>>();
    lower_capsules(&capsules, checked.graph.root)
}

pub(crate) fn lower_capsules(
    capsules: &[&typecheck::CheckedSourceModule],
    root: Option<usize>,
) -> Result<Vec<(String, String)>, CompileFailure> {
    if root.is_some_and(|index| index >= capsules.len()) {
        return Err(CompileFailure::InvalidOptions(
            "the root module is outside the capsule inventory".to_owned(),
        ));
    }
    let program = crate::mono::Program::new(capsules.iter().copied());
    let mut skipped = Vec::new();
    let mut texts = Vec::with_capacity(capsules.len());
    for (index, module) in capsules.iter().enumerate() {
        let lowered = if root == Some(index) {
            crate::lower_next::lower_program(&program, module)
        } else {
            crate::lower_next::lower_library(&program, module, &mut skipped)
        };
        let text = lowered.map_err(|error| CompileFailure::Lowering(error.to_string()))?;
        texts.push((module.name().to_owned(), text));
    }
    for left_out in &skipped {
        let called = format!("{}(", left_out.symbol);
        if texts.iter().any(|(_, text)| text.contains(&called)) {
            return Err(CompileFailure::Lowering(left_out.error.clone()));
        }
    }
    Ok(texts)
}

/// Compile each module's text to an object file at the path `path_of(index, module name)`.
fn compile_objects(
    texts: &[(String, String)],
    options: BuildOptions<'_>,
    timings: &mut CompilationTimings,
    mut path_of: impl FnMut(usize, &str) -> PathBuf,
) -> Result<Vec<PathBuf>, CompileFailure> {
    let compiler = NativeCompiler::default();
    let mut objects = Vec::with_capacity(texts.len());
    // One row for all the modules: how many there are is not what the table is for.
    let start = std::time::Instant::now();
    for (index, (name, text)) in texts.iter().enumerate() {
        let object = path_of(index, name);
        let (emitted, diagnostics) = compiler.native_from_mlir_text_with(
            text,
            &object,
            options.profile,
            options.fast_llvm,
            options.target,
        );
        if !emitted {
            timings.record("object", start.elapsed());
            return Err(CompileFailure::EmissionFailed {
                stage: CompileEmitStage::ObjectEmission,
                diagnostics,
            });
        }
        objects.push(object);
    }
    timings.record("object", start.elapsed());
    assert_eq!(objects.len(), texts.len());
    Ok(objects)
}

pub(crate) fn build_executable(
    checked: &CheckedPackage,
    output: &Path,
    options: BuildOptions<'_>,
    timings: &mut CompilationTimings,
) -> Result<(), CompileFailure> {
    if checked.entry().is_none() {
        return Err(CompileFailure::Package(
            "the root module has no `#entry` function to build".to_owned(),
        ));
    }
    let texts = timings.run("lower", || lower_package(checked))?;
    let scratch = tempfile::tempdir().map_err(CompileFailure::ArtifactStorage)?;
    let objects = compile_objects(&texts, options, timings, |index, _| {
        scratch.path().join(format!("module{index}.o"))
    })?;
    let paths: Vec<&Path> = objects.iter().map(|object| object.as_path()).collect();
    let (linked, diagnostics) = timings.run("link", || {
        NativeCompiler::link_executables(&paths, output, options.target)
    });
    match linked {
        true => Ok(()),
        false => Err(CompileFailure::EmissionFailed {
            stage: CompileEmitStage::Linking,
            diagnostics,
        }),
    }
}

/// One object per module in `directory`, named `<module>.o`; nothing is linked.
pub(crate) fn build_objects(
    checked: &CheckedPackage,
    directory: &Path,
    options: BuildOptions<'_>,
    timings: &mut CompilationTimings,
) -> Result<(), CompileFailure> {
    let texts = timings.run("lower", || lower_package(checked))?;
    std::fs::create_dir_all(directory).map_err(CompileFailure::ArtifactStorage)?;
    compile_objects(&texts, options, timings, |_, name| {
        directory.join(format!("{name}.o"))
    })?;
    Ok(())
}
