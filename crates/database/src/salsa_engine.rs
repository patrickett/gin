use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::Sender;
use notify_debouncer_mini::DebounceEventResult;

use crate::input_database::InputDatabase;
use crate::package::{intern_package_files, sorted_package_files};
use crate::queries::{file_parse_output, set_file_contents};
use crate::semantic_queries::{hover_markdown, package_typecheck_symptoms};
use crate::{Db, File, QueryEngine};

/// Salsa-backed [`QueryEngine`] adapter.
///
/// Wraps [`InputDatabase`] and delegates every method to Salsa-tracked
/// queries. Callers never touch `salsa`, `File`, `PackageFiles`, or
/// `Diagnostics` directly.
pub struct SalsaQueryEngine {
    db: InputDatabase,
    /// Reverse map: File → PathBuf so we can look up by path.
    files: HashMap<PathBuf, File>,
}

impl SalsaQueryEngine {
    pub fn new(tx: Sender<DebounceEventResult>) -> Self {
        Self {
            db: InputDatabase::new(tx),
            files: HashMap::new(),
        }
    }

    pub fn new_with_debug_logging(tx: Sender<DebounceEventResult>, debug: bool) -> Self {
        Self {
            db: InputDatabase::new_with_debug_logging(tx, debug),
            files: HashMap::new(),
        }
    }
}

impl QueryEngine for SalsaQueryEngine {
    fn add_file(&mut self, path: PathBuf) -> Result<(), String> {
        // Try to load from disk. If the file doesn't exist yet (e.g. unsaved
        // editor buffer), create it with empty contents.
        let file = match self.db.input(path.clone()) {
            Ok(f) => f,
            Err(_) => {
                let f = crate::File::new(&self.db, path.clone(), String::new());
                self.db.files.insert(path.clone(), f);
                f
            }
        };
        self.files.insert(path, file);
        Ok(())
    }

    fn set_contents(&mut self, path: &Path, contents: String) {
        if let Some(file) = self.files.get(path) {
            set_file_contents(&mut self.db, *file, contents);
        }
    }

    fn source_and_parse(&self, path: &Path) -> Option<(String, Arc<parser::ParseOutput>)> {
        let file = self.files.get(path)?;
        let source = file.contents(&self.db).to_string();
        let parse = file_parse_output(&self.db, *file);
        Some((source, parse))
    }

    fn parse_output(&self, path: &Path) -> Option<Arc<parser::ParseOutput>> {
        let file = self.files.get(path)?;
        Some(file_parse_output(&self.db, *file))
    }

    fn typecheck_package(&self, paths: &[PathBuf]) -> Vec<Vec<diagnostic::Diagnostic>> {
        if paths.is_empty() {
            return Vec::new();
        }
        let package_files: Vec<File> = paths
            .iter()
            .filter_map(|p| self.files.get(p).copied())
            .collect();
        if package_files.is_empty() {
            return Vec::new();
        }
        let sorted = sorted_package_files(&self.db, &package_files);
        let pkg = intern_package_files(&self.db, sorted.clone());
        let symptoms = package_typecheck_symptoms(&self.db, pkg);
        // Re-map back to the caller's path order
        let mut by_path: HashMap<PathBuf, Vec<diagnostic::Diagnostic>> = HashMap::new();
        for (i, &file) in sorted.iter().enumerate() {
            let p = file.path(&self.db);
            by_path.entry(p).or_default().extend(symptoms[i].clone());
        }
        paths
            .iter()
            .map(|p| by_path.remove(p).unwrap_or_default())
            .collect()
    }

    fn hover(&self, path: &Path, byte_pos: u32) -> Option<String> {
        let file = self.files.get(path)?;
        hover_markdown(&self.db, *file, byte_pos)
    }

    fn file_paths(&self) -> Vec<PathBuf> {
        self.files.keys().cloned().collect()
    }

    fn contains(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn snapshot(&self) -> Box<dyn QueryEngine> {
        Box::new(Self {
            db: self.db.clone(),
            files: self.files.clone(),
        })
    }
}
