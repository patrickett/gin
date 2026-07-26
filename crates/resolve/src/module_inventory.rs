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
}

#[cfg(test)]
mod tests {
    use super::ModuleInventory;
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn discovers_and_caches_only_requested_module_directory() {
        let root =
            std::env::temp_dir().join(format!("gin_module_inventory_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("core/primitive")).unwrap();
        fs::create_dir_all(root.join("core/target")).unwrap();
        let primitive = root.join("core/primitive/bool.gin");
        let target = root.join("core/target/arch.gin");
        fs::write(&primitive, "Bool is True or False\n").unwrap();
        fs::write(&target, "Architecture is 'x86_64'\n").unwrap();

        let mut inventory = ModuleInventory::discover(&HashMap::new());
        let primitive_module = root.join("core/primitive");

        assert_eq!(
            inventory.module_files(&primitive_module),
            vec![primitive.clone()]
        );
        fs::remove_file(&primitive).unwrap();
        assert_eq!(inventory.module_files(&primitive_module), vec![primitive]);
        assert!(target.is_file());
        let _ = fs::remove_dir_all(root);
    }
}
