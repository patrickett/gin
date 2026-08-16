use crate::PACKAGE_CONFIG_NAME;
use std::path::{Path, PathBuf};

/// Result of resolving `use dep.seg1.seg2...` to a folder on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NestedPackageTarget {
    /// Directory containing `flask.jsonc` (a folder module / nested package root).
    FolderModule(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NestedPackageResolveError {
    MissingConfig {
        dir: PathBuf,
    },
    /// Expected `parent/segment/` to be a directory with `flask.jsonc`.
    NestedPackageNotFound {
        parent: PathBuf,
        segment: String,
    },
    /// A non-final segment exists but is not a folder module.
    IntermediateNotFolderModule {
        path: PathBuf,
    },
}

/// Extension trait providing Flask-package path utilities on [`Path`].
pub trait FlaskPathExt {
    /// Whether `self` is a directory containing `flask.jsonc` (a folder module / nested package root).
    fn is_folder_module_dir(&self) -> bool;

    /// All `*.gin` files directly in `self` (non-recursive). Sorted for stable builds.
    fn list_package_gin_files(&self) -> Vec<PathBuf>;

    /// Resolve `use dep.seg1.seg2` as nested directories under `self` (the resolved dependency root).
    ///
    /// For each segment in order, the path must be `previous/segment/` with a `flask.jsonc` in that
    /// directory. The last segment's directory is returned; there is no resolution to a single `.gin` file.
    fn resolve_nested_package_path(
        &self,
        segments: &[impl AsRef<str>],
    ) -> Result<NestedPackageTarget, NestedPackageResolveError>;
}

impl FlaskPathExt for Path {
    fn is_folder_module_dir(&self) -> bool {
        self.is_dir() && self.join(PACKAGE_CONFIG_NAME).is_file()
    }

    fn list_package_gin_files(&self) -> Vec<PathBuf> {
        // TODO: exclude `*.private.gin` from the public package surface once private modules are implemented.
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(self) else {
            return out;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().is_some_and(|ext| ext == "gin") {
                out.push(p);
            }
        }
        out.sort();
        out
    }

    fn resolve_nested_package_path(
        &self,
        segments: &[impl AsRef<str>],
    ) -> Result<NestedPackageTarget, NestedPackageResolveError> {
        if segments.is_empty() {
            return Err(NestedPackageResolveError::NestedPackageNotFound {
                parent: self.to_path_buf(),
                segment: "<empty>".to_string(),
            });
        }

        let mut dir = self.to_path_buf();
        if !dir.is_folder_module_dir() {
            return Err(NestedPackageResolveError::MissingConfig { dir: dir.clone() });
        }

        for (i, seg) in segments.iter().enumerate() {
            let key = seg.as_ref();
            let next = dir.join(key);
            let is_last = i + 1 == segments.len();

            if is_last {
                if next.is_folder_module_dir() {
                    return Ok(NestedPackageTarget::FolderModule(next));
                }
                return Err(NestedPackageResolveError::NestedPackageNotFound {
                    parent: dir,
                    segment: key.to_string(),
                });
            }

            if !next.is_folder_module_dir() {
                return Err(NestedPackageResolveError::IntermediateNotFolderModule { path: next });
            }
            dir = next;
        }

        Err(NestedPackageResolveError::NestedPackageNotFound {
            parent: dir,
            segment: "<empty>".to_string(),
        })
    }
}
