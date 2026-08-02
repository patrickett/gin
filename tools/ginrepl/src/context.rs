use flask::{CompileTarget, FlaskConfig};
use resolve::{GinPackageExt, ParsedFile};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct ReplPackage {
    root: PathBuf,
    dependencies: HashMap<String, PathBuf>,
    files: Vec<ParsedFile>,
    session_path: PathBuf,
}

impl ReplPackage {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn dependencies(&self) -> &HashMap<String, PathBuf> {
        &self.dependencies
    }

    pub(crate) fn files(&self) -> &[ParsedFile] {
        &self.files
    }

    pub(crate) fn session_path(&self) -> &Path {
        &self.session_path
    }
}

#[derive(Clone)]
pub struct ReplContext {
    package: ReplPackage,
    target: CompileTarget,
}

impl ReplContext {
    pub fn isolated() -> Self {
        let root = std::env::current_dir().unwrap_or_default();
        Self {
            package: ReplPackage {
                session_path: root.join(".gin-repl.gin"),
                root,
                dependencies: HashMap::new(),
                files: Vec::new(),
            },
            target: CompileTarget::Library,
        }
    }

    pub fn load(path: &Path) -> Result<Self, ReplContextError> {
        let path = path
            .canonicalize()
            .map_err(|error| ReplContextError::InvalidPath {
                path: path.to_path_buf(),
                error: error.to_string(),
            })?;
        let search_dir = if path.is_dir() {
            path.as_path()
        } else {
            path.parent().unwrap_or(path.as_path())
        };
        let (config, root) = FlaskConfig::find_package_config(search_dir)
            .ok_or_else(|| ReplContextError::PackageNotFound(path.clone()))?;
        let target = CompileTarget::resolve(&config, None)
            .map_err(|error| ReplContextError::InvalidTarget(error.to_string()))?;
        let dependencies = root.resolve_flask_dependencies(&config);
        let files = root
            .collect_gin_files()
            .into_iter()
            .map(|path| ParsedFile::read(&path).ok_or(ReplContextError::UnreadableSource(path)))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            package: ReplPackage {
                session_path: root.join(".gin-repl.gin"),
                root,
                dependencies,
                files,
            },
            target,
        })
    }

    pub fn package(&self) -> &ReplPackage {
        &self.package
    }

    pub fn target(&self) -> &CompileTarget {
        &self.target
    }
}

#[derive(Debug)]
pub enum ReplContextError {
    InvalidPath { path: PathBuf, error: String },
    PackageNotFound(PathBuf),
    InvalidTarget(String),
    UnreadableSource(PathBuf),
}

impl fmt::Display for ReplContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath { path, error } => {
                write!(formatter, "cannot open {}: {error}", path.display())
            }
            Self::PackageNotFound(path) => write!(
                formatter,
                "no flask.jsonc found for {} or its parents",
                path.display()
            ),
            Self::InvalidTarget(error) => write!(formatter, "invalid package target: {error}"),
            Self::UnreadableSource(path) => {
                write!(formatter, "cannot read Gin source {}", path.display())
            }
        }
    }
}

impl std::error::Error for ReplContextError {}
