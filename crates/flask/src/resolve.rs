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
    MissingConfig { dir: PathBuf },
    /// Expected `parent/segment/` to be a directory with `flask.jsonc`.
    NestedPackageNotFound { parent: PathBuf, segment: String },
    /// A non-final segment exists but is not a folder module.
    IntermediateNotFolderModule { path: PathBuf },
}

fn is_folder_module_dir(path: &Path) -> bool {
    path.is_dir() && path.join(PACKAGE_CONFIG_NAME).is_file()
}

/// All `*.gin` files directly in `package_dir` (non-recursive). Sorted for stable builds.
pub fn list_package_gin_files(package_dir: &Path) -> Vec<PathBuf> {
    // TODO: exclude `*.private.gin` from the public package surface once private modules are implemented.
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(package_dir) else {
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

/// Resolve `use dep.seg1.seg2` as nested directories under `start_dir` (the resolved dependency root).
///
/// For each segment in order, the path must be `previous/segment/` with a `flask.jsonc` in that
/// directory. The last segment’s directory is returned; there is no resolution to a single `.gin` file.
pub fn resolve_nested_package_path(
    start_dir: &Path,
    segments: &[impl AsRef<str>],
) -> Result<NestedPackageTarget, NestedPackageResolveError> {
    if segments.is_empty() {
        return Err(NestedPackageResolveError::NestedPackageNotFound {
            parent: start_dir.to_path_buf(),
            segment: "<empty>".to_string(),
        });
    }

    let mut dir = start_dir.to_path_buf();
    if !is_folder_module_dir(&dir) {
        return Err(NestedPackageResolveError::MissingConfig { dir: dir.clone() });
    }

    for (i, seg) in segments.iter().enumerate() {
        let key = seg.as_ref();
        let next = dir.join(key);
        let is_last = i + 1 == segments.len();

        if is_last {
            if is_folder_module_dir(&next) {
                return Ok(NestedPackageTarget::FolderModule(next));
            }
            return Err(NestedPackageResolveError::NestedPackageNotFound {
                parent: dir,
                segment: key.to_string(),
            });
        }

        if !is_folder_module_dir(&next) {
            return Err(NestedPackageResolveError::IntermediateNotFolderModule { path: next });
        }
        dir = next;
    }

    Err(NestedPackageResolveError::NestedPackageNotFound {
        parent: dir,
        segment: "<empty>".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "gin_flask_resolve_test_{}_{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn nested_path_two_segments_requires_flask_at_each_level() {
        let root = tmp("two_seg");
        fs::create_dir_all(&root).unwrap();
        let dep = root.join("dep");
        fs::create_dir_all(dep.join("seg1/seg2")).unwrap();
        fs::write(dep.join("flask.jsonc"), "{}").unwrap();
        fs::write(dep.join("seg1/flask.jsonc"), "{}").unwrap();
        fs::write(dep.join("seg1/seg2/flask.jsonc"), "{}").unwrap();

        let t = resolve_nested_package_path(&dep, &["seg1", "seg2"]).unwrap();
        match t {
            NestedPackageTarget::FolderModule(p) => assert_eq!(p, dep.join("seg1/seg2")),
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn nested_path_missing_intermediate_flask_errors() {
        let root = tmp("missing_mid");
        fs::create_dir_all(&root).unwrap();
        let dep = root.join("dep");
        fs::create_dir_all(dep.join("seg1/seg2")).unwrap();
        fs::write(dep.join("flask.jsonc"), "{}").unwrap();
        fs::write(dep.join("seg1/seg2/flask.jsonc"), "{}").unwrap();

        let err = resolve_nested_package_path(&dep, &["seg1", "seg2"]).unwrap_err();
        assert!(matches!(
            err,
            NestedPackageResolveError::IntermediateNotFolderModule { .. }
        ));
        let _ = fs::remove_dir_all(&root);
    }
}

