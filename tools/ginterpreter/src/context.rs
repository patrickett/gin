use flask::{CompileTarget, FlaskConfig, TargetTriple};
use ginc::driver::{
    CheckedPackage, CompilePackage, CompilerDriver, CompilerDriverError, SourceFile, SourcePackage,
};
use resolve::GinPackageExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Clone)]
pub struct InterpreterPackage {
    root: PathBuf,
    dependencies: HashMap<String, PathBuf>,
    sources: PackageSources,
    session_path: PathBuf,
}

#[derive(Clone)]
enum PackageSources {
    Isolated,
    Files(Vec<SourceFile>),
}

impl InterpreterPackage {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn dependencies(&self) -> &HashMap<String, PathBuf> {
        &self.dependencies
    }

    pub(crate) fn sources(&self) -> &[SourceFile] {
        match &self.sources {
            PackageSources::Isolated => &[],
            PackageSources::Files(sources) => sources,
        }
    }

    pub(crate) fn session_path(&self) -> &Path {
        &self.session_path
    }
}

#[derive(Clone)]
pub struct InterpreterContext {
    package: InterpreterPackage,
    target: CompileTarget,
}

impl InterpreterContext {
    pub fn isolated() -> Self {
        let root = std::env::current_dir().unwrap_or_default();
        Self::isolated_root(root)
    }

    pub fn isolated_at(path: &Path) -> Result<Self, InterpreterContextError> {
        let root = context_directory(path)?;
        Ok(Self::isolated_root(root))
    }

    pub fn discover(path: &Path) -> Result<Self, InterpreterContextError> {
        let root = context_directory(path)?;
        let package =
            FlaskConfig::discover(&root).map_err(InterpreterContextError::InvalidPackage)?;
        match package {
            Some((config, root)) => Self::load_package(config, root),
            None => Ok(Self::isolated_root(root)),
        }
    }

    fn isolated_root(root: PathBuf) -> Self {
        Self {
            package: InterpreterPackage {
                session_path: root.join(".gin-interpreter.gin"),
                root,
                dependencies: HashMap::new(),
                sources: PackageSources::Isolated,
            },
            target: native_compile_target(),
        }
    }

    pub fn load(path: &Path) -> Result<Self, InterpreterContextError> {
        let root = context_directory(path)?;
        let (config, root) = FlaskConfig::discover(&root)
            .map_err(InterpreterContextError::InvalidPackage)?
            .ok_or(InterpreterContextError::PackageNotFound(root))?;
        Self::load_package(config, root)
    }

    fn load_package(config: FlaskConfig, root: PathBuf) -> Result<Self, InterpreterContextError> {
        let target = CompileTarget::resolve(&config, None)
            .map_err(|error| InterpreterContextError::InvalidTarget(error.to_string()))?;
        let target = match target {
            CompileTarget::Library => native_compile_target(),
            target => target,
        };
        let dependencies = root.resolve_flask_dependencies(&config);
        let session_path = root.join(".gin-interpreter.gin");
        let sources = read_package_sources(&root, &session_path)?;

        Ok(Self {
            package: InterpreterPackage {
                session_path,
                root,
                dependencies,
                sources: PackageSources::Files(sources),
            },
            target,
        })
    }

    pub fn package(&self) -> &InterpreterPackage {
        &self.package
    }

    pub fn target(&self) -> &CompileTarget {
        &self.target
    }

    pub fn is_isolated(&self) -> bool {
        matches!(self.package.sources, PackageSources::Isolated)
    }

    pub(crate) fn reload(&self) -> Result<Self, InterpreterContextError> {
        match &self.package.sources {
            PackageSources::Isolated => Ok(self.clone()),
            PackageSources::Files(_) => Self::load(&self.package.root),
        }
    }

    pub(crate) fn check_source(
        &self,
        driver: &mut CompilerDriver,
        source: &str,
        mut sources: Vec<SourceFile>,
    ) -> Result<CheckedPackage, CompilerDriverError> {
        let session_path = self.package.session_path().to_path_buf();
        sources.push(SourceFile::new(session_path.clone(), source));
        let parsed = driver.parse(SourcePackage::new(session_path, sources))?;
        let package = CompilePackage::new(parsed, self.target.clone())
            .with_dependencies(self.package.dependencies().clone())
            .with_import_resolution(!self.is_isolated());
        Ok(driver.check(package))
    }
}

fn context_directory(path: &Path) -> Result<PathBuf, InterpreterContextError> {
    let path = path
        .canonicalize()
        .map_err(|error| InterpreterContextError::InvalidPath {
            path: path.to_path_buf(),
            error: error.to_string(),
        })?;
    if path.is_dir() {
        Ok(path)
    } else {
        Ok(path.parent().unwrap_or(&path).to_path_buf())
    }
}

fn read_package_sources(
    root: &Path,
    session_path: &Path,
) -> Result<Vec<SourceFile>, InterpreterContextError> {
    root.collect_gin_files()
        .into_iter()
        .filter(|path| path != session_path)
        .map(|path| match std::fs::read_to_string(&path) {
            Ok(source) => Ok(SourceFile::new(path, source)),
            Err(_) => Err(InterpreterContextError::UnreadableSource(path)),
        })
        .collect()
}

fn native_compile_target() -> CompileTarget {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        arch => panic!("unsupported interpreter host architecture: {arch}"),
    };
    let suffix = match std::env::consts::OS {
        "linux" => "unknown-linux-gnu",
        "macos" => "apple-darwin",
        "windows" => "pc-windows-msvc",
        os => panic!("unsupported interpreter host operating system: {os}"),
    };
    let triple = TargetTriple::parse(&format!("{arch}-{suffix}"))
        .expect("interpreter host target must be supported");
    CompileTarget::Concrete(triple)
}

#[derive(Debug, Error)]
pub enum InterpreterContextError {
    #[error("cannot open {path}: {error}")]
    InvalidPath { path: PathBuf, error: String },
    #[error("no mod.flask found for {0} or its parents")]
    PackageNotFound(PathBuf),
    #[error("invalid package configuration: {0}")]
    InvalidPackage(String),
    #[error("invalid package target: {0}")]
    InvalidTarget(String),
    #[error("cannot read Gin source {0}")]
    UnreadableSource(PathBuf),
}

#[cfg(test)]
#[path = "tests/context_tests.rs"]
mod tests;
