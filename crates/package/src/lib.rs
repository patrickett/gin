//! Folder-module loading for the source-AST pipeline.
//!
//! A folder module is a directory under the package root; its immediate `*.gin` files,
//! in sorted order, are the files of one module. Subdirectories are separate modules.
//! Nothing here type checks: the files come back parsed with file-naming span ids,
//! ready for `typecheck::check_module_files`.

mod check;
mod graph;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use diagnostic::Diagnostic;
use flask::{DependencyKind, FlaskConfig, MANIFEST_NAME};
use ast::{SourceFile, SpanId};

pub use check::{CheckedPackage, PackageError};
pub use graph::{GraphError, GraphLimits, PackageGraph};

const SOURCE_EXTENSION: &str = "gin";
const FILE_BYTES_MAX_DEFAULT: u32 = 16 * 1024 * 1024;

/// Bounds on one module load. `files_max` can never exceed what a checker can address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageLimits {
    pub files_max: u32,
    pub file_bytes_max: u32,
}

impl PackageLimits {
    /// Files of one module that span ids can name, keeping `SpanId::FILES_MAX` free.
    pub const FILES_MAX: u32 = SpanId::FILES_MAX - 1;
}

impl Default for PackageLimits {
    fn default() -> Self {
        Self {
            files_max: Self::FILES_MAX,
            file_bytes_max: FILE_BYTES_MAX_DEFAULT,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("package manifest {path} was not found")]
    ManifestMissing { path: PathBuf },
    #[error("package manifest {path} is invalid: {message}")]
    ManifestInvalid { path: PathBuf, message: String },
    #[error("module path segment {segment:?} is not a plain folder name")]
    SegmentInvalid { segment: String },
    #[error("module `{module}` has no folder at {path}")]
    ModuleNotFound { module: String, path: PathBuf },
    #[error("module `{module}` has no .gin files")]
    NoSourceFiles { module: String },
    #[error("module `{module}` has {count} files; at most {max} are allowed")]
    TooManyFiles {
        module: String,
        count: usize,
        max: u32,
    },
    #[error("{path} has {bytes} bytes; at most {max} are allowed")]
    FileTooLarge { path: PathBuf, bytes: u64, max: u32 },
    #[error("could not read {path}: {message}")]
    Read { path: PathBuf, message: String },
    #[error("{path} has syntax errors")]
    Syntax {
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
    #[error("package limits are invalid: {message}")]
    LimitsInvalid { message: &'static str },
}

/// Where a dependency named in a manifest lives, as far as this loader can follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyRoot {
    /// A package in this directory.
    Path(PathBuf),
    /// A kind this loader does not resolve (`"registry"` or `"git"`).
    Unsupported(&'static str),
}

/// An opened package: its root directory, the module name its manifest declares, and the
/// dependencies it names, by the name an import uses.
#[derive(Debug, Clone)]
pub struct Package {
    root: PathBuf,
    name: String,
    dependencies: Vec<(String, DependencyRoot)>,
    /// Texts that stand in for files on disk, keyed by canonical path (an editor's unsaved
    /// buffers). Empty for a command-line check.
    overlay: Arc<BTreeMap<PathBuf, String>>,
}

/// One module's files, parsed in sorted-path order; `files[i]` was parsed as file `i`.
#[derive(Debug)]
pub struct LoadedModule {
    pub name: String,
    pub directory: PathBuf,
    pub paths: Vec<PathBuf>,
    pub files: Vec<SourceFile>,
}

impl Package {
    pub fn open(root: &Path) -> Result<Self, LoadError> {
        let manifest = FlaskConfig::manifest_path(root);
        let raw = fs::read_to_string(&manifest).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => LoadError::ManifestMissing {
                path: manifest.clone(),
            },
            _ => LoadError::Read {
                path: manifest.clone(),
                message: error.to_string(),
            },
        })?;
        let config =
            FlaskConfig::parse_manifest(&raw).map_err(|error| LoadError::ManifestInvalid {
                path: manifest.clone(),
                message: error.to_string(),
            })?;
        assert!(manifest.ends_with(MANIFEST_NAME));
        let mut dependencies: Vec<(String, DependencyRoot)> = config
            .dependencies()
            .iter()
            .map(|(name, dependency)| {
                let root = match &dependency.kind {
                    DependencyKind::Path { path } => DependencyRoot::Path(root.join(path)),
                    DependencyKind::Version { .. } => DependencyRoot::Unsupported("registry"),
                    DependencyKind::Git { .. } => DependencyRoot::Unsupported("git"),
                    // A kind added later is unsupported until this loader learns it.
                    _ => DependencyRoot::Unsupported("unknown"),
                };
                (name.clone(), root)
            })
            .collect();
        dependencies.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(Self {
            root: root.to_owned(),
            name: config.name,
            dependencies,
            overlay: Arc::default(),
        })
    }

    /// This package reading `overlay` (canonical path to text) instead of the file on disk
    /// wherever a path is in it. Dependencies opened later share the overlay. The files that
    /// make up a module are still the ones on disk.
    #[must_use]
    pub fn with_overlay(mut self, overlay: Arc<BTreeMap<PathBuf, String>>) -> Self {
        self.overlay = overlay;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The dependency this package's manifest names `name`, if it names one.
    pub fn dependency(&self, name: &str) -> Option<&DependencyRoot> {
        self.dependencies
            .iter()
            .find(|(other, _)| other == name)
            .map(|(_, root)| root)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Load the module at `segments` below the root; no segments is the root module.
    pub fn load_module(
        &self,
        segments: &[&str],
        limits: PackageLimits,
    ) -> Result<LoadedModule, LoadError> {
        if limits.files_max == 0 || limits.files_max > PackageLimits::FILES_MAX {
            return Err(LoadError::LimitsInvalid {
                message: "files_max must be between 1 and PackageLimits::FILES_MAX",
            });
        }
        let name = self.module_name(segments);
        let directory = self.module_directory(segments, &name)?;
        let paths = source_paths(&directory, &name, limits)?;
        let mut files = Vec::with_capacity(paths.len());
        for (index, path) in paths.iter().enumerate() {
            files.push(parse_path(path, index, limits, &self.overlay)?);
        }
        assert_eq!(files.len(), paths.len());
        Ok(LoadedModule {
            name,
            directory,
            paths,
            files,
        })
    }

    fn module_name(&self, segments: &[&str]) -> String {
        let mut name = self.name.clone();
        for segment in segments {
            name.push('.');
            name.push_str(segment);
        }
        name
    }

    fn module_directory(&self, segments: &[&str], name: &str) -> Result<PathBuf, LoadError> {
        let mut directory = self.root.clone();
        for segment in segments {
            if !plain_segment(segment) {
                return Err(LoadError::SegmentInvalid {
                    segment: (*segment).to_owned(),
                });
            }
            directory.push(segment);
        }
        if !directory.is_dir() {
            return Err(LoadError::ModuleNotFound {
                module: name.to_owned(),
                path: directory,
            });
        }
        Ok(directory)
    }
}

fn plain_segment(segment: &str) -> bool {
    !segment.is_empty() && segment != "." && segment != ".." && !segment.contains(['/', '\\', '\0'])
}

/// Immediate `.gin` files of `directory`, sorted, with the file-count limit enforced.
fn source_paths(
    directory: &Path,
    module: &str,
    limits: PackageLimits,
) -> Result<Vec<PathBuf>, LoadError> {
    let read = |error: std::io::Error| LoadError::Read {
        path: directory.to_owned(),
        message: error.to_string(),
    };
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(read)? {
        let path = entry.map_err(read)?.path();
        let is_source = path.extension().is_some_and(|ext| ext == SOURCE_EXTENSION);
        if is_source && path.is_file() {
            paths.push(path);
            if paths.len() > limits.files_max as usize {
                return Err(LoadError::TooManyFiles {
                    module: module.to_owned(),
                    count: paths.len(),
                    max: limits.files_max,
                });
            }
        }
    }
    if paths.is_empty() {
        return Err(LoadError::NoSourceFiles {
            module: module.to_owned(),
        });
    }
    paths.sort();
    Ok(paths)
}

fn parse_path(
    path: &Path,
    index: usize,
    limits: PackageLimits,
    overlay: &BTreeMap<PathBuf, String>,
) -> Result<SourceFile, LoadError> {
    assert!(index < limits.files_max as usize);
    let read = |error: std::io::Error| LoadError::Read {
        path: path.to_owned(),
        message: error.to_string(),
    };
    let replaced = if overlay.is_empty() {
        None
    } else {
        fs::canonicalize(path).ok().and_then(|canonical| overlay.get(&canonical))
    };
    let bytes = match replaced {
        Some(text) => text.len() as u64,
        None => fs::metadata(path).map_err(read)?.len(),
    };
    if bytes > u64::from(limits.file_bytes_max) {
        return Err(LoadError::FileTooLarge {
            path: path.to_owned(),
            bytes,
            max: limits.file_bytes_max,
        });
    }
    let text = match replaced {
        Some(text) => text.clone(),
        None => fs::read_to_string(path).map_err(read)?,
    };
    parser::parse_strict_file(&text, index as u32).map_err(|diagnostics| LoadError::Syntax {
        path: path.to_owned(),
        diagnostics,
    })
}
