use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use parser::query::SourceParseExt;

use crate::ParsedFile;
use crate::module_inventory::ModuleInventory;

#[derive(Default)]
pub struct ParsedModuleCache {
    pub(crate) files: HashMap<PathBuf, ParsedFile>,
}

impl ParsedModuleCache {
    pub fn remove_file(&mut self, path: &Path) -> Option<ParsedFile> {
        self.files.remove(path)
    }

    pub fn has_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    pub fn get_file(&self, path: &Path) -> Option<&ParsedFile> {
        self.files.get(path)
    }
}

pub(crate) struct ModuleLoader {
    inventory: ModuleInventory,
    parsed_files: HashMap<PathBuf, ParsedFile>,
}

impl ModuleLoader {
    #[cfg(test)]
    pub(crate) fn new(
        inventory: ModuleInventory,
        entry_files: impl IntoIterator<Item = ParsedFile>,
    ) -> Self {
        Self::with_cache(inventory, entry_files, ParsedModuleCache::default())
    }

    pub(crate) fn with_cache(
        inventory: ModuleInventory,
        entry_files: impl IntoIterator<Item = ParsedFile>,
        mut cache: ParsedModuleCache,
    ) -> Self {
        for file in entry_files {
            cache.files.insert(file.path.clone(), file);
        }
        Self {
            inventory,
            parsed_files: cache.files,
        }
    }

    pub(crate) fn load_module(&mut self, module_dir: &Path) -> Vec<PathBuf> {
        let mut paths = self.inventory.module_files(module_dir);
        paths.extend(
            self.parsed_files
                .keys()
                .filter(|path| path.parent() == Some(module_dir))
                .cloned(),
        );
        paths.sort();
        paths.dedup();
        let missing: Vec<PathBuf> = paths
            .iter()
            .filter(|path| !self.parsed_files.contains_key(*path))
            .cloned()
            .collect();
        if missing.is_empty() {
            return paths;
        }

        let (tx, rx) = mpsc::channel();
        let handles: Vec<_> = missing
            .iter()
            .map(|path| {
                let path = path.clone();
                let tx = tx.clone();
                thread::spawn(move || {
                    let parsed = std::fs::read_to_string(&path).ok().map(|source| {
                        (
                            path.clone(),
                            ParsedFile {
                                path: path.clone(),
                                output: source.parse_source_full(),
                                source,
                            },
                        )
                    });
                    let _ = tx.send(parsed);
                })
            })
            .collect();
        drop(tx);

        for handle in handles {
            let _ = handle.join();
        }
        for (path, file) in rx.into_iter().flatten() {
            self.parsed_files.entry(path).or_insert(file);
        }
        paths
    }

    pub(crate) fn find_public_def(&mut self, module_dir: &Path, symbol: &str) -> Option<PathBuf> {
        let mut paths = self.load_module(module_dir);
        paths.sort();
        for path in paths {
            let file = self.parsed_files.get(&path)?;
            let name = internment::Intern::<String>::from_ref(symbol);
            if (!file.output.ast.private_defs.contains(&name)
                && file.output.ast.defs.contains_key(&name))
                || (!file.output.ast.private_tags.contains(&name)
                    && file.output.ast.tags.contains_key(&name))
            {
                return Some(path);
            }
        }
        None
    }

    pub(crate) fn load_containing_module(&mut self, path: &Path) {
        if let Some(module_dir) = path.parent() {
            self.load_module(module_dir);
        }
    }

    pub(crate) fn module_files(&mut self, module_dir: &Path) -> Vec<PathBuf> {
        self.load_module(module_dir)
    }

    pub(crate) fn parsed_file(&self, path: &Path) -> Option<&ParsedFile> {
        self.parsed_files.get(path)
    }

    pub(crate) fn parsed_file_cloned(&self, path: &Path) -> Option<ParsedFile> {
        self.parsed_files.get(path).cloned()
    }

    pub(crate) fn into_cache(self) -> ParsedModuleCache {
        ParsedModuleCache {
            files: self.parsed_files,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ModuleLoader;
    use crate::module_inventory::ModuleInventory;
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn reuses_cached_parse_after_source_is_removed() {
        let root =
            std::env::temp_dir().join(format!("gin_module_loader_cache_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let core = root.join("core");
        fs::create_dir_all(core.join("primitive")).unwrap();
        fs::write(core.join("flask.jsonc"), "{}").unwrap();
        let bool_path = core.join("primitive/bool.gin");
        fs::write(&bool_path, "Bool is True or False\n").unwrap();

        let mut dependencies = HashMap::new();
        dependencies.insert("core".to_string(), core.clone());
        let inventory = ModuleInventory::discover(&dependencies);
        let mut loader = ModuleLoader::new(inventory, []);
        let primitive = core.join("primitive");

        assert_eq!(
            loader.find_public_def(&primitive, "Bool"),
            Some(bool_path.clone())
        );
        fs::remove_file(&bool_path).unwrap();
        assert_eq!(loader.find_public_def(&primitive, "Bool"), Some(bool_path));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_only_the_requested_module() {
        let root = std::env::temp_dir().join(format!("gin_module_loader_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let core = root.join("core");
        fs::create_dir_all(core.join("primitive")).unwrap();
        fs::create_dir_all(core.join("target")).unwrap();
        fs::write(core.join("flask.jsonc"), "{}").unwrap();
        let bool_path = core.join("primitive/bool.gin");
        let target_path = core.join("target/arch.gin");
        fs::write(&bool_path, "Bool is True or False\n").unwrap();
        fs::write(&target_path, "Architecture is 'x86_64'\n").unwrap();

        let mut dependencies = HashMap::new();
        dependencies.insert("core".to_string(), core.clone());
        let inventory = ModuleInventory::discover(&dependencies);
        let mut loader = ModuleLoader::new(inventory, []);

        assert_eq!(
            loader.find_public_def(&core.join("primitive"), "Bool"),
            Some(bool_path)
        );
        assert!(loader.parsed_file(&target_path).is_none());
        let _ = fs::remove_dir_all(root);
    }
}
