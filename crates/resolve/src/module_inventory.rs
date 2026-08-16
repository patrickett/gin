use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(crate) struct ModuleInventory {
    module_files: HashMap<PathBuf, Vec<PathBuf>>,
}

impl ModuleInventory {
    pub(crate) fn discover(_dependencies: &HashMap<String, PathBuf>) -> Self {
        Self {
            module_files: HashMap::new(),
        }
    }

    pub(crate) fn module_files(&mut self, module_dir: &Path) -> Vec<PathBuf> {
        if let Some(files) = self.module_files.get(module_dir) {
            return files.clone();
        }

        let files: Vec<PathBuf> = module_dir
            .read_dir()
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|ext| ext == "gin") {
                    return Some(path);
                }
                None
            })
            .collect();
        let mut files = files;
        files.sort();
        files.dedup();
        self.module_files
            .insert(module_dir.to_path_buf(), files.clone());
        files
    }

    fn contains_gin_files(&mut self, module_dir: &Path) -> bool {
        if !self.module_files(module_dir).is_empty() {
            return true;
        }
        let Ok(entries) = std::fs::read_dir(module_dir) else {
            return false;
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .any(|path| path.is_dir() && self.contains_gin_files(&path))
    }
}

pub fn child_folder_module_names(package_dir: &Path) -> Vec<String> {
    let mut inventory = ModuleInventory::discover(&HashMap::new());
    let Ok(entries) = std::fs::read_dir(package_dir) else {
        return Vec::new();
    };
    let mut names: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.is_dir() && inventory.contains_gin_files(&path))
                .then(|| entry.file_name().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    names
}
#[cfg(test)]
#[path = "../tests/module_inventory_tests.rs"]
mod tests;
