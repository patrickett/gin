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

pub(crate) struct ModuleLoader {
    inventory: ModuleInventory,
    parsed_files: HashMap<PathBuf, ParsedFile>,
}

impl ModuleLoader {
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

        let profile_context = compiler_profile::Context::current();
        let (tx, rx) = mpsc::channel();
        let handles: Vec<_> = missing
            .iter()
            .map(|path| {
                let path = path.clone();
                let tx = tx.clone();
                let profile_context = profile_context.clone();
                thread::spawn(move || {
                    let _profile_context = profile_context
                        .as_ref()
                        .map(compiler_profile::Context::enter);
                    let source = {
                        let _read = compiler_profile::span(compiler_profile::Phase::Read);
                        std::fs::read_to_string(&path)
                    };
                    let parsed = source.ok().map(|source| {
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
            if file.output.ast.contains_public_symbol(&name) {
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
