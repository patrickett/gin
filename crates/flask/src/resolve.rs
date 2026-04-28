use crate::{FlaskConfig, PACKAGE_CONFIG_NAME};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportTarget {
    /// A concrete file path (usually `*.gin`).
    File(PathBuf),
    /// A folder-module directory path (contains `flask.jsonc`).
    FolderModule(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportResolveError {
    MissingConfig { dir: PathBuf },
    MissingExport { dir: PathBuf, key: String },
    IntermediateNotFolderModule { path: PathBuf },
}

fn is_folder_module_dir(path: &Path) -> bool {
    path.is_dir() && path.join(PACKAGE_CONFIG_NAME).is_file()
}

/// Resolve `segments` by repeatedly looking up `exports[key].path`.
///
/// Semantics:
/// - Intermediate segments must resolve to folder-modules (dir + `flask.jsonc`), so we can continue.
/// - The final segment may resolve to either a file or a folder-module.
pub fn resolve_chained_exports(
    start_dir: &Path,
    segments: &[impl AsRef<str>],
) -> Result<ExportTarget, ExportResolveError> {
    let mut dir = start_dir.to_path_buf();
    let mut cfg = FlaskConfig::from_directory(&dir).ok_or_else(|| ExportResolveError::MissingConfig {
        dir: dir.clone(),
    })?;

    for (i, seg) in segments.iter().enumerate() {
        let key = seg.as_ref();
        let spec = cfg
            .exports()
            .get(key)
            .ok_or_else(|| ExportResolveError::MissingExport {
                dir: dir.clone(),
                key: key.to_string(),
            })?;

        let next = dir.join(&spec.path);
        let is_last = i + 1 == segments.len();
        if is_last {
            if is_folder_module_dir(&next) {
                return Ok(ExportTarget::FolderModule(next));
            }
            return Ok(ExportTarget::File(next));
        }

        if !is_folder_module_dir(&next) {
            return Err(ExportResolveError::IntermediateNotFolderModule { path: next });
        }

        dir = next;
        cfg = FlaskConfig::from_directory(&dir).ok_or_else(|| ExportResolveError::MissingConfig {
            dir: dir.clone(),
        })?;
    }

    // Caller should only use this for non-empty segments.
    Err(ExportResolveError::MissingExport {
        dir,
        key: "<empty>".to_string(),
    })
}

