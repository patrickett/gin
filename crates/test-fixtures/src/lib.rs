//! Shared temporary Package / Flask fixtures for integration tests.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

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
        self.write(
            "flask.jsonc",
            &format!(r#"{{"name":"{name}","version":"0.0.0","authors":[]}}"#),
        );
    }

    pub fn write_flask_with_deps(&self, name: &str, deps_json: &str) {
        self.write(
            "flask.jsonc",
            &format!(
                r#"{{"name":"{name}","version":"0.0.0","authors":[],"dependencies":{deps_json}}}"#
            ),
        );
    }

    pub fn write_nested_flask(&self, relative_dir: &str, name: &str) {
        let d = self.root.join(relative_dir);
        fs::create_dir_all(&d).unwrap();
        write_file(
            &d.join("flask.jsonc"),
            &format!(r#"{{"name":"{name}","version":"0.0.0","authors":[]}}"#),
        );
    }
}

impl Drop for TempPackage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
