use flask::{CompileTarget, FlaskConfig, TargetTriple};
use ginc::driver::SourceFile;
use resolve::GinPackageExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Clone)]
pub struct InterpreterPackage {
    root: PathBuf,
    dependencies: HashMap<String, PathBuf>,
    sources: Vec<SourceFile>,
    session_path: PathBuf,
}

impl InterpreterPackage {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn dependencies(&self) -> &HashMap<String, PathBuf> {
        &self.dependencies
    }

    pub(crate) fn sources(&self) -> &[SourceFile] {
        &self.sources
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
        Self {
            package: InterpreterPackage {
                session_path: root.join(".gin-interpreter.gin"),
                root,
                dependencies: HashMap::new(),
                sources: Vec::new(),
            },
            target: native_compile_target(),
        }
    }

    pub fn load(path: &Path) -> Result<Self, InterpreterContextError> {
        let path = path
            .canonicalize()
            .map_err(|error| InterpreterContextError::InvalidPath {
                path: path.to_path_buf(),
                error: error.to_string(),
            })?;
        let search_dir = if path.is_dir() {
            path.as_path()
        } else {
            path.parent().unwrap_or(path.as_path())
        };
        let (config, root) = FlaskConfig::find_package_config(search_dir)
            .ok_or_else(|| InterpreterContextError::PackageNotFound(path.clone()))?;
        let target = CompileTarget::resolve(&config, None)
            .map_err(|error| InterpreterContextError::InvalidTarget(error.to_string()))?;
        let target = match target {
            CompileTarget::Library => native_compile_target(),
            target => target,
        };
        let dependencies = root.resolve_flask_dependencies(&config);
        let session_path = root.join(".gin-interpreter.gin");
        let sources = root
            .collect_gin_files()
            .into_iter()
            .filter(|path| path != &session_path)
            .map(|path| match std::fs::read_to_string(&path) {
                Ok(source) => Ok(SourceFile::new(path, source)),
                Err(_) => Err(InterpreterContextError::UnreadableSource(path)),
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            package: InterpreterPackage {
                session_path,
                root,
                dependencies,
                sources,
            },
            target,
        })
    }

    pub fn gin_core() -> Result<Self, InterpreterContextError> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../modules/gin_core")
            .canonicalize()
            .map_err(|error| InterpreterContextError::InvalidPath {
                path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../modules/gin_core"),
                error: error.to_string(),
            })?;
        let source_paths = [root.join("happy.gin"), root.join("primitive/int.gin")];
        let sources = source_paths
            .into_iter()
            .map(|path| match std::fs::read_to_string(&path) {
                Ok(source) => Ok(SourceFile::new(path, source)),
                Err(_) => Err(InterpreterContextError::UnreadableSource(path)),
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            package: InterpreterPackage {
                session_path: root.join(".gin-interpreter.gin"),
                dependencies: HashMap::from([("core".to_string(), root.clone())]),
                sources,
                root,
            },
            target: native_compile_target(),
        })
    }

    pub fn package(&self) -> &InterpreterPackage {
        &self.package
    }

    pub fn target(&self) -> &CompileTarget {
        &self.target
    }
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
    #[error("no flask.jsonc found for {0} or its parents")]
    PackageNotFound(PathBuf),
    #[error("invalid package target: {0}")]
    InvalidTarget(String),
    #[error("cannot read Gin source {0}")]
    UnreadableSource(PathBuf),
}
