//! Logical folder-module discovery within a single `flask.jsonc` package root.
//!
//! A **folder module** is a directory under the package root. Only immediate `*.gin`
//! files in that directory contribute to its public symbol surface; subdirectories
//! are separate folder modules addressed by path segments.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizePathError {
    EscapesPackageRoot {
        path: PathBuf,
        package_root: PathBuf,
    },
    NotADirectory(PathBuf),
    ImportTargetMustBeFolder {
        path: PathBuf,
    },
}

/// Normalize a quoted import path relative to `importing_file_dir`, fenced at `package_root`.
pub fn normalize_local_import_path(
    importing_file_dir: &Path,
    package_root: &Path,
    import_path: &Path,
) -> Result<PathBuf, NormalizePathError> {
    let joined = importing_file_dir.join(import_path);
    let mut normalized = normalize_path_components(&joined);

    if normalized.is_file() {
        return Err(NormalizePathError::ImportTargetMustBeFolder { path: normalized });
    }

    if let Ok(canon_root) = package_root.canonicalize() {
        if let Ok(canon) = normalized.canonicalize() {
            if !canon.starts_with(&canon_root) {
                return Err(NormalizePathError::EscapesPackageRoot {
                    path: canon,
                    package_root: canon_root,
                });
            }
            normalized = canon;
        } else if !path_starts_with(&normalized, package_root) {
            return Err(NormalizePathError::EscapesPackageRoot {
                path: normalized.clone(),
                package_root: package_root.to_path_buf(),
            });
        }
    } else if !path_starts_with(&normalized, package_root) {
        return Err(NormalizePathError::EscapesPackageRoot {
            path: normalized.clone(),
            package_root: package_root.to_path_buf(),
        });
    }

    if normalized.is_dir() {
        Ok(normalized)
    } else if normalized.with_extension("gin").is_file() {
        Err(NormalizePathError::ImportTargetMustBeFolder { path: normalized })
    } else {
        Err(NormalizePathError::NotADirectory(normalized))
    }
}

fn normalize_path_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}

fn path_starts_with(path: &Path, prefix: &Path) -> bool {
    path.components().zip(prefix.components()).count() >= prefix.components().count()
        && path
            .components()
            .zip(prefix.components())
            .all(|(a, b)| a == b)
}

/// Resolve a logical folder module under `package_root` from slash/dot path segments.
pub fn resolve_logical_module_dir(package_root: &Path, segments: &[&str]) -> Option<PathBuf> {
    let mut dir = package_root.to_path_buf();
    for seg in segments {
        if seg.is_empty() {
            continue;
        }
        dir = dir.join(seg);
        if !dir.is_dir() {
            return None;
        }
    }
    Some(dir)
}

/// Split path segments for dependency imports: module path vs trailing member symbol.
///
/// `http.client.get` → module `["client"]`, member `Some("get")` when `get` is not a subdir.
pub fn split_dep_path_segments(
    dep_root: &Path,
    segments: &[impl AsRef<str>],
) -> (Vec<String>, Option<String>) {
    if segments.is_empty() {
        return (vec![], None);
    }

    let mut module_segs: Vec<String> = Vec::new();
    let mut i = 0;
    while i < segments.len() {
        let seg = segments[i].as_ref().to_string();
        let refs: Vec<&str> = module_segs
            .iter()
            .chain(std::iter::once(&seg))
            .map(|s| s.as_str())
            .collect();
        let next_dir = resolve_logical_module_dir(dep_root, &refs);
        if next_dir.is_some() {
            module_segs.push(seg);
            i += 1;
        } else {
            break;
        }
    }

    if i < segments.len() {
        let member = segments[i].as_ref().to_string();
        if i + 1 < segments.len() {
            // Remaining segments are invalid; treat whole tail as module attempt.
            for segment in segments.iter().skip(i) {
                module_segs.push(segment.as_ref().to_string());
            }
            return (module_segs, None);
        }
        return (module_segs, Some(member));
    }

    (module_segs, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("gin_folder_mod_test_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn mkdir(&self, rel: &str) {
            fs::create_dir_all(self.path.join(rel)).unwrap();
        }

        fn write(&self, rel: &str, contents: &str) {
            let p = self.path.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(p, contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn normalize_fences_package_root() {
        let tmp = TempDir::new("fence");
        tmp.write("flask.jsonc", "{}");
        tmp.mkdir("sub");
        tmp.write("sub/a.gin", "x := 1\n");

        let pkg = tmp.path.canonicalize().unwrap();
        let sub_dir = pkg.join("sub");
        let err = normalize_local_import_path(&sub_dir, &pkg, Path::new("../..")).unwrap_err();
        assert!(matches!(err, NormalizePathError::EscapesPackageRoot { .. }));
    }

    #[test]
    fn rejects_gin_file_as_import_target() {
        let tmp = TempDir::new("file_reject");
        tmp.write("flask.jsonc", "{}");
        tmp.write("a.gin", "x := 1\n");

        let pkg = tmp.path.canonicalize().unwrap();
        let err = normalize_local_import_path(&pkg, &pkg, Path::new("a.gin")).unwrap_err();
        assert!(matches!(
            err,
            NormalizePathError::ImportTargetMustBeFolder { .. }
        ));
    }

    #[test]
    fn split_dep_path_member_vs_module() {
        let tmp = TempDir::new("split");
        tmp.write("flask.jsonc", "{}");
        tmp.mkdir("client");
        tmp.write("client/get.gin", "get := 1\n");
        tmp.write("root.gin", "help := 1\n");

        let pkg = tmp.path.canonicalize().unwrap();
        let (mods, mem) = split_dep_path_segments(&pkg, &["client", "get"]);
        assert_eq!(mods, vec!["client"]);
        assert_eq!(mem, Some("get".to_string()));
    }
}
