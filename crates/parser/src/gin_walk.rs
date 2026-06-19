//! Gitignore-aware `.gin` file discovery.

use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

/// Extension trait providing gitignore-aware `.gin` file discovery on [`Path`].
pub trait GinPathExt {
    /// Whether `self` is excluded by gitignore rules (including parent `.gitignore` files).
    fn is_gitignored(&self) -> bool;

    /// Recursively collect `.gin` files under `self`, honoring `.gitignore`.
    fn collect_gin_files_under(&self) -> Vec<PathBuf>;

    /// Build a `WalkBuilder` configured for gin file discovery.
    fn walk_builder(&self) -> WalkBuilder;

    /// Whether the path's file name starts with `.`.
    fn is_hidden(&self) -> bool;
}

impl GinPathExt for Path {
    fn is_gitignored(&self) -> bool {
        if self.is_hidden() {
            return true;
        }
        let Some(parent) = self.parent() else {
            return false;
        };
        let Some(name) = self.file_name() else {
            return false;
        };
        !parent
            .walk_builder()
            .max_depth(Some(1))
            .build()
            .flatten()
            .any(|entry| entry.file_name() == name)
    }

    fn collect_gin_files_under(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in self.walk_builder().build().flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "gin") {
                files.push(path.to_path_buf());
            }
        }
        files.sort();
        files.dedup();
        files
    }

    fn walk_builder(&self) -> WalkBuilder {
        let mut builder = WalkBuilder::new(self);
        builder
            .require_git(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .hidden(true);
        builder
    }

    fn is_hidden(&self) -> bool {
        self.file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.starts_with('.'))
    }
}
