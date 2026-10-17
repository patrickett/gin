use std::collections::HashMap;
use std::path::PathBuf;

use ast::ty::PackageInstanceKey;
use diagnostic::Diagnostic;
use flask::CompileTarget;
use interface::PublicInterface;
use typecheck::TypedFileAst;
use typecheck::transform::{
    BodyReuse, PackageCheckSession, PackageTransformOptions,
    transform_package_with_shared_context_and_package,
    transform_package_with_shared_context_and_session,
};

use crate::package_resolver::resolve_imports_with_home;
use crate::{ImportDependencyGraph, ParsedFile, ParsedModuleCache};

pub struct PackageFrontEndOptions {
    pub gin_home: Option<flask::GinHome>,
    pub compile_target: CompileTarget,
    pub transform: PackageTransformOptions,
    pub resolve_imports: bool,
    pub package_fallback: PackageInstanceKey,
    pub transform_paths: Option<Vec<PathBuf>>,
}

pub struct FrontEndFile {
    pub parsed: ParsedFile,
    pub typed: Option<TypedFileAst>,
    pub import_diagnostics: Vec<Diagnostic>,
    pub prepare_diagnostics: Vec<Diagnostic>,
}

pub struct PackageFrontEndOutput {
    pub files: Vec<FrontEndFile>,
    pub public_interface: PublicInterface,
    pub cache: ParsedModuleCache,
    pub import_graph: Option<ImportDependencyGraph>,
}

/// A [`PackageFrontEndOutput`] with every file's typed AST unwrapped.
///
/// The native compiler and the editor both build their products from this, so
/// their typed ASTs, public interface and semantic index cannot drift apart.
/// The downstream per-file diagnostics differ between the two and stay with each
/// consumer.
pub struct AssembledPackage {
    pub files: Vec<AssembledFile>,
    pub public_interface: PublicInterface,
    pub import_graph: Option<ImportDependencyGraph>,
    pub cache: ParsedModuleCache,
}

pub struct AssembledFile {
    pub parsed: ParsedFile,
    pub typed: TypedFileAst,
    pub import_diagnostics: Vec<Diagnostic>,
    pub prepare_diagnostics: Vec<Diagnostic>,
}

impl AssembledPackage {
    /// Panics if a file was not checked. Every caller transforms the whole
    /// package (no `transform_paths`), so a missing typed AST is a bug.
    pub fn assemble(front_end: PackageFrontEndOutput) -> Self {
        let PackageFrontEndOutput {
            files,
            public_interface,
            cache,
            import_graph,
        } = front_end;
        let files = files
            .into_iter()
            .map(|file| AssembledFile {
                typed: file
                    .typed
                    .expect("the package front end checks every transformed file"),
                parsed: file.parsed,
                import_diagnostics: file.import_diagnostics,
                prepare_diagnostics: file.prepare_diagnostics,
            })
            .collect();
        Self {
            files,
            public_interface,
            cache,
            import_graph,
        }
    }

    /// Cross-file tag/variant index for package-scoped hover and completion.
    pub fn semantic_index(&self) -> typecheck::PackageSemanticIndex {
        typecheck::PackageSemanticIndex::from_typed_asts(
            &self
                .files
                .iter()
                .map(|file| &file.typed)
                .collect::<Vec<_>>(),
        )
    }
}

pub struct PreparedPackageFrontEnd {
    files: Vec<FrontEndFile>,
    options: PackageTransformOptions,
    package: PackageInstanceKey,
    transform_paths: Option<Vec<PathBuf>>,
    cache: ParsedModuleCache,
    import_graph: Option<ImportDependencyGraph>,
}

pub struct PackageFrontEndInputs {
    files: Vec<(ParsedFile, Vec<Diagnostic>, Vec<Diagnostic>)>,
    options: PackageTransformOptions,
    package: PackageInstanceKey,
    transform_paths: Option<Vec<PathBuf>>,
}

impl PackageFrontEndInputs {
    pub fn matches(&self, prepared: &PreparedPackageFrontEnd) -> bool {
        let PackageTransformOptions {
            ide_package,
            compile_target,
        } = &self.options;
        *ide_package == prepared.options.ide_package
            && *compile_target == prepared.options.compile_target
            && self.package == prepared.package
            && self.transform_paths == prepared.transform_paths
            && self.files.len() == prepared.files.len()
            && self.files.iter().zip(&prepared.files).all(
                |((parsed, imports, preparation), file)| {
                    parsed.path == file.parsed.path
                        && parsed.source == file.parsed.source
                        && parsed.output == file.parsed.output
                        && parsed
                            .output
                            .ast
                            .defs
                            .keys()
                            .eq(file.parsed.output.ast.defs.keys())
                        && parsed.output.ast.defs.groups() == file.parsed.output.ast.defs.groups()
                        && imports == &file.import_diagnostics
                        && preparation == &file.prepare_diagnostics
                },
            )
    }
}

impl PreparedPackageFrontEnd {
    pub fn input_snapshot(&self) -> PackageFrontEndInputs {
        PackageFrontEndInputs {
            files: self
                .files
                .iter()
                .map(|file| {
                    (
                        file.parsed.clone(),
                        file.import_diagnostics.clone(),
                        file.prepare_diagnostics.clone(),
                    )
                })
                .collect(),
            options: self.options.clone(),
            package: self.package.clone(),
            transform_paths: self.transform_paths.clone(),
        }
    }

    pub fn into_dependency_state(self) -> (ParsedModuleCache, Option<ImportDependencyGraph>) {
        (self.cache, self.import_graph)
    }
}

pub fn run_package_front_end(
    files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
    cache: ParsedModuleCache,
    options: PackageFrontEndOptions,
) -> PackageFrontEndOutput {
    check_package_front_end(
        prepare_package_front_end(files, dependencies, cache, options),
        None,
    )
}

/// The single check-session configuration shared by every production caller —
/// the native compiler and the editor (`ginlsp`) — so their checking policy
/// cannot drift apart.
///
pub fn default_check_session() -> Option<PackageCheckSession> {
    let one = std::num::NonZeroUsize::MIN;
    PackageCheckSession::new(one, BodyReuse::ExactInputs)
        .ok()
        .map(|session| {
            session
                .with_retained_pass_limit(one)
                .with_finishing_reuse()
                .with_effect_queries()
                .with_function_reuse()
        })
}

pub fn run_package_front_end_with_session(
    files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
    cache: ParsedModuleCache,
    options: PackageFrontEndOptions,
    session: &mut PackageCheckSession,
) -> PackageFrontEndOutput {
    check_package_front_end(
        prepare_package_front_end(files, dependencies, cache, options),
        Some(session),
    )
}

pub fn prepare_package_front_end(
    files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
    cache: ParsedModuleCache,
    options: PackageFrontEndOptions,
) -> PreparedPackageFrontEnd {
    let _profile = compiler_profile::span(compiler_profile::Phase::Prepare);
    let (files, cache, import_graph) = if options.resolve_imports {
        let resolved =
            resolve_imports_with_home(files, dependencies, cache, options.gin_home.as_ref());
        let mut files_by_path: HashMap<PathBuf, ParsedFile> = resolved
            .files
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect();
        let file_paths = files_by_path.keys().cloned().collect::<Vec<_>>();
        let ordered_paths = resolved.import_graph.declaration_file_order(&file_paths);
        let ordered_files = ordered_paths
            .into_iter()
            .filter_map(|path| files_by_path.remove(&path))
            .collect();
        (ordered_files, resolved.cache, Some(resolved.import_graph))
    } else {
        let mut files = files;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        (files, cache, None)
    };

    let files: Vec<_> = files
        .into_iter()
        .map(|mut parsed| {
            let import_diagnostics = if import_graph.is_some() {
                let parse_count = cache.files[&parsed.path].output.symptoms.len();
                parsed.output.symptoms.split_off(parse_count)
            } else {
                Vec::new()
            };
            let prepare_diagnostics =
                typecheck::prepare_parse_ast(&mut parsed.output.ast, &options.compile_target);
            FrontEndFile {
                parsed,
                typed: None,
                import_diagnostics,
                prepare_diagnostics,
            }
        })
        .collect();
    let package = files
        .iter()
        .find_map(|file| file.parsed.output.ast.semantic_origin.as_ref())
        .map(|origin| origin.package.clone())
        .unwrap_or(options.package_fallback);
    let transform_paths = options.transform_paths.map(|paths| {
        paths
            .into_iter()
            .map(|path| {
                if options.resolve_imports {
                    path.canonicalize().unwrap_or(path)
                } else {
                    path
                }
            })
            .collect::<Vec<_>>()
    });
    PreparedPackageFrontEnd {
        files,
        options: options
            .transform
            .with_compile_target(options.compile_target),
        package,
        transform_paths,
        cache,
        import_graph,
    }
}

pub fn check_package_front_end(
    prepared: PreparedPackageFrontEnd,
    session: Option<&mut PackageCheckSession>,
) -> PackageFrontEndOutput {
    let _profile = compiler_profile::span(compiler_profile::Phase::FrontEnd);
    let PreparedPackageFrontEnd {
        mut files,
        options,
        package,
        transform_paths,
        cache,
        import_graph,
    } = prepared;
    let selected: Vec<_> = files
        .iter_mut()
        .filter(|file| {
            transform_paths
                .as_ref()
                .is_none_or(|paths| paths.contains(&file.parsed.path))
        })
        .collect();
    let clone_profile = compiler_profile::span(compiler_profile::Phase::AstClone);
    let transform_asts = selected
        .iter()
        .map(|file| file.parsed.output.ast.clone())
        .collect();
    drop(clone_profile);
    let artifacts = match session {
        Some(session) => transform_package_with_shared_context_and_session(
            transform_asts,
            options,
            package,
            session,
        ),
        None => transform_package_with_shared_context_and_package(transform_asts, options, package),
    };

    assert_eq!(selected.len(), artifacts.typed_asts.len());
    for (file, typed) in selected.into_iter().zip(artifacts.typed_asts) {
        file.typed = Some(typed);
    }

    PackageFrontEndOutput {
        files,
        public_interface: artifacts.public_interface,
        cache,
        import_graph,
    }
}
