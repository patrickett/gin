//! Shared temporary Package / Flask fixtures for integration tests.

#![allow(dead_code)]

pub mod bench_sources;
pub mod emission;
pub mod preludes;
pub mod semantic_fixtures;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn host_target_triple() -> String {
    flask::TargetTriple::host()
        .expect("test host must have a supported target triple")
        .raw
}

/// The host triple as a concrete compile target, for native codegen tests.
pub fn host_compile_target() -> flask::CompileTarget {
    flask::CompileTarget::Concrete(flask::TargetTriple::parse(&host_target_triple()).unwrap())
}

/// Unique directory under the system temp dir (parallel-safe).
pub fn unique_temp_dir(prefix: &str) -> PathBuf {
    let id = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    std::env::temp_dir().join(format!(
        "gin_{prefix}_{}_{}_{}",
        std::process::id(),
        nanos,
        id
    ))
}

/// Write `contents` to `path`, creating parent directories.
pub fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Temporary Gin Package with automatic cleanup on drop.
pub struct TempPackage {
    pub root: PathBuf,
}

impl TempPackage {
    pub fn new(name: &str) -> Self {
        let root = unique_temp_dir(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
        self.root.join(rel)
    }

    pub fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let p = self.root.join(rel);
        write_file(&p, contents);
        p
    }

    pub fn write_flask(&self, name: &str) {
        self.write_flask_with_deps(name, "");
    }

    pub fn write_flask_with_deps(&self, name: &str, dependencies: &str) {
        let manifest =
            format!("#settings\nmodule: '{name}'\nversion: 0.0.0\n\n#dependencies\n{dependencies}");
        flask::FlaskConfig::parse_manifest(&manifest).unwrap();
        self.write(flask::MANIFEST_NAME, &manifest);
    }

    pub fn write_nested_flask(&self, relative_dir: &str, name: &str) {
        let manifest = format!("#settings\nmodule: '{name}'\nversion: 0.0.0\n");
        flask::FlaskConfig::parse_manifest(&manifest).unwrap();
        self.write(
            &format!("{relative_dir}/{}", flask::MANIFEST_NAME),
            &manifest,
        );
    }

    /// Copy `.gin` and `mod.flask` files from `src_root` under `prefix`,
    /// mirroring the subdirectory layout.
    pub fn copy_tree(&self, src_root: &Path, prefix: &str) {
        let mut pending = vec![src_root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| extension == "gin")
                    || path.file_name().is_some_and(|file| file == "mod.flask")
                {
                    let relative = Path::new(prefix).join(path.strip_prefix(src_root).unwrap());
                    self.write(
                        relative.to_str().unwrap(),
                        &fs::read_to_string(path).unwrap(),
                    );
                }
            }
        }
    }
}

impl Drop for TempPackage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn with_runtime(source: &str) -> String {
    format!("{source}\n{}", semantic_fixtures::RUNTIME)
}
