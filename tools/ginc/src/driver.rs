use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

use ast::ty::PackageInstanceKey;
use codegen::prelude::{Context, Module};
use codegen::{CodegenContext, CodegenSourceMap, ResolvedFileInput};
use diagnostic::{Category, Diagnostic};
use flask::CompileTarget;
use interface::PublicInterface;
use parser::query::SourceParseExt;
use resolve::{ImportDependencyGraph, ParsedFile, ParsedModuleCache};
use typecheck::transform::PackageTransformOptions;
use typecheck::{PackageSemanticIndex, TypedFileAst};

#[derive(Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub source: String,
}

impl SourceFile {
    pub fn new(path: impl Into<PathBuf>, source: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            source: source.into(),
        }
    }
}

pub struct SourcePackage {
    primary_path: PathBuf,
    sources: Vec<SourceFile>,
}

impl SourcePackage {
    pub fn new(primary_path: impl Into<PathBuf>, sources: Vec<SourceFile>) -> Self {
        Self {
            primary_path: primary_path.into(),
            sources,
        }
    }
}

pub struct ParsedPackage {
    primary_path: PathBuf,
    files: Vec<ParsedFile>,
}

impl ParsedPackage {
    pub fn files(&self) -> &[ParsedFile] {
        &self.files
    }

    pub fn has_flaws(&self) -> bool {
        self.files.iter().any(|file| {
            file.output
                .symptoms
                .iter()
                .any(|diagnostic| diagnostic.category == Category::Flaw)
        })
    }
}

pub struct CompilePackage {
    gin_home: Option<flask::GinHome>,
    parsed: ParsedPackage,
    dependencies: HashMap<String, PathBuf>,
    target: CompileTarget,
    resolve_imports: bool,
    package_fallback: PackageInstanceKey,
}

impl CompilePackage {
    pub fn new(parsed: ParsedPackage, target: CompileTarget) -> Self {
        Self {
            parsed,
            gin_home: None,
            dependencies: HashMap::new(),
            target,
            resolve_imports: false,
            package_fallback: PackageInstanceKey::workspace("anonymous", "0"),
        }
    }

    pub fn with_gin_home(mut self, home: Option<flask::GinHome>) -> Self {
        self.gin_home = home;
        self
    }

    pub fn with_dependencies(mut self, dependencies: HashMap<String, PathBuf>) -> Self {
        self.dependencies = dependencies;
        self
    }

    pub fn with_import_resolution(mut self, resolve_imports: bool) -> Self {
        self.resolve_imports = resolve_imports;
        self
    }

    pub fn with_package_fallback(mut self, package_fallback: PackageInstanceKey) -> Self {
        self.package_fallback = package_fallback;
        self
    }
}

#[derive(Debug, Error)]
pub enum CompilerDriverError {
    #[error("primary source {path} was not provided")]
    PrimarySourceNotFound { path: PathBuf },
    #[error("source path {path} was provided more than once")]
    DuplicateSourcePath { path: PathBuf },
}

pub struct CheckedFile {
    parsed: ParsedFile,
    typed: TypedFileAst,
    typecheck_diagnostics: Vec<Diagnostic>,
}

impl CheckedFile {
    pub fn path(&self) -> &Path {
        &self.parsed.path
    }

    pub fn source(&self) -> &str {
        &self.parsed.source
    }

    pub fn parsed(&self) -> &ParsedFile {
        &self.parsed
    }

    pub fn typed(&self) -> &TypedFileAst {
        &self.typed
    }

    pub fn front_end_diagnostics(&self) -> &[Diagnostic] {
        &self.parsed.output.symptoms
    }

    pub fn typecheck_diagnostics(&self) -> &[Diagnostic] {
        &self.typecheck_diagnostics
    }

    pub fn has_front_end_flaws(&self) -> bool {
        self.front_end_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.category == Category::Flaw)
    }

    pub fn has_typecheck_flaws(&self) -> bool {
        !self.typecheck_diagnostics.is_empty()
    }
}

pub struct CheckedPackage {
    primary_index: usize,
    files: Vec<CheckedFile>,
    import_graph: Option<ImportDependencyGraph>,
    target: CompileTarget,
    public_interface: PublicInterface,
    package_index: PackageSemanticIndex,
}

impl CheckedPackage {
    pub fn primary_path(&self) -> &Path {
        self.primary_file().path()
    }

    pub fn files(&self) -> &[CheckedFile] {
        &self.files
    }

    pub fn target(&self) -> &CompileTarget {
        &self.target
    }

    pub fn file(&self, path: &Path) -> Option<&CheckedFile> {
        self.files.iter().find(|file| file.path() == path)
    }

    pub fn primary_file(&self) -> &CheckedFile {
        &self.files[self.primary_index]
    }

    pub fn has_front_end_flaws(&self) -> bool {
        self.files.iter().any(CheckedFile::has_front_end_flaws)
    }

    pub fn has_typecheck_flaws(&self) -> bool {
        self.files.iter().any(CheckedFile::has_typecheck_flaws)
    }

    pub fn import_graph(&self) -> Option<&ImportDependencyGraph> {
        self.import_graph.as_ref()
    }

    pub fn public_interface(&self) -> &PublicInterface {
        &self.public_interface
    }

    pub fn package_index(&self) -> &PackageSemanticIndex {
        &self.package_index
    }
}

pub struct LoweredFileDiagnostics {
    pub path: PathBuf,
    pub diagnostics: Vec<Diagnostic>,
}

pub struct LoweredPackage<'c> {
    pub module: Option<Module<'c>>,
    pub diagnostics: Vec<LoweredFileDiagnostics>,
}

#[derive(Default)]
pub struct CompilerDriver {
    cache: ParsedModuleCache,
}

impl CompilerDriver {
    pub fn parse(&self, package: SourcePackage) -> Result<ParsedPackage, CompilerDriverError> {
        let mut paths = HashSet::with_capacity(package.sources.len());
        for source in &package.sources {
            if !paths.insert(&source.path) {
                return Err(CompilerDriverError::DuplicateSourcePath {
                    path: source.path.clone(),
                });
            }
        }
        if !package
            .sources
            .iter()
            .any(|source| source.path == package.primary_path)
        {
            return Err(CompilerDriverError::PrimarySourceNotFound {
                path: package.primary_path,
            });
        }

        let files = package
            .sources
            .into_iter()
            .map(|source| ParsedFile {
                path: source.path,
                output: source.source.parse_source_full(),
                source: source.source,
            })
            .collect();

        Ok(ParsedPackage {
            primary_path: package.primary_path,
            files,
        })
    }

    pub fn check(&mut self, package: CompilePackage) -> CheckedPackage {
        let primary_path = package.parsed.primary_path;
        let target = package.target.clone();
        let front_end = resolve::run_package_front_end(
            package.parsed.files,
            &package.dependencies,
            std::mem::take(&mut self.cache),
            resolve::PackageFrontEndOptions {
                gin_home: package.gin_home,
                compile_target: target.clone(),
                transform: PackageTransformOptions::FULL,
                resolve_imports: package.resolve_imports,
                package_fallback: package.package_fallback,
                transform_paths: None,
            },
        );
        self.cache = front_end.cache;
        let files: Vec<_> = front_end
            .files
            .into_iter()
            .map(|file| {
                let mut parsed = file.parsed;
                parsed.output.symptoms.extend(file.import_diagnostics);
                parsed.output.symptoms.extend(file.prepare_diagnostics);
                let typed = file
                    .typed
                    .expect("full package transformation checks every file");
                let typecheck_diagnostics = typed
                    .all_flaws()
                    .into_iter()
                    .map(|(span, diagnostic)| {
                        if diagnostic.span.start() == 0 && diagnostic.span.end() == 0 {
                            diagnostic.clone().at_span_id(span, &typed.span_table)
                        } else {
                            diagnostic.clone()
                        }
                    })
                    .collect();
                CheckedFile {
                    parsed,
                    typed,
                    typecheck_diagnostics,
                }
            })
            .collect();
        let package_index = PackageSemanticIndex::from_typed_asts(
            &files.iter().map(CheckedFile::typed).collect::<Vec<_>>(),
        );
        let primary_index = files
            .iter()
            .position(|file| file.path() == primary_path)
            .expect("package frontend retains the validated primary source");

        CheckedPackage {
            primary_index,
            files,
            import_graph: front_end.import_graph,
            target,
            public_interface: front_end.public_interface,
            package_index,
        }
    }

    pub fn lower_package<'c>(
        &self,
        context: &'c Context,
        package: &CheckedPackage,
    ) -> LoweredPackage<'c> {
        let inputs: Vec<_> = package
            .files()
            .iter()
            .map(|file| {
                ResolvedFileInput::new(
                    file.typed().resolved_program(),
                    CodegenSourceMap::new(
                        file.path().to_string_lossy(),
                        file.source(),
                        &file.typed().span_table,
                    ),
                )
            })
            .collect();
        let output = CodegenContext::build_module_from_resolved_files_for_target(
            context,
            &inputs,
            &package.target,
        );
        assert_eq!(
            package.files().len(),
            output.diagnostics.len(),
            "codegen returned unpaired file diagnostics"
        );
        let diagnostics = package
            .files()
            .iter()
            .zip(output.diagnostics)
            .map(|(file, diagnostics)| LoweredFileDiagnostics {
                path: file.path().to_path_buf(),
                diagnostics: diagnostics.diagnostics,
            })
            .collect();
        LoweredPackage {
            module: output.module,
            diagnostics,
        }
    }
}
#[cfg(test)]
#[path = "tests/driver_tests.rs"]
mod tests;
