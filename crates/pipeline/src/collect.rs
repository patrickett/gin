use std::path::{Path, PathBuf};

use crate::SourceCollection;

/// Collect all `.gin` file paths under `root`, skipping `target/` directories.
///
/// If `root` is a directory, all `.gin` files are discovered recursively (library mode).
/// If `root` is a file, only that file is included (binary mode).
pub fn collect(root: &Path) -> SourceCollection {
    let is_library = root.is_dir();
    let file_paths = if is_library {
        collect_gin_files_recursive(root)
    } else {
        vec![root.to_path_buf()]
    };
    SourceCollection {
        file_paths,
        is_library,
    }
}

fn collect_gin_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            files.extend(collect_gin_files_recursive(&path));
        } else if path.extension().is_some_and(|ext| ext == "gin") {
            files.push(path);
        }
    }

    files
}
