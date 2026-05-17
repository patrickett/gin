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
        let file_path = file.path(&self.db);

        // Build a cross-file TransformCtx from all registered files in the same
        // package, so that imported types (e.g. `List`, `Byte`) resolve correctly.
        if let Some(pkg_root) = file_path.parent().and_then(find_package_root) {
            let mut pkg_files: Vec<File> = self
                .files
                .values()
                .filter(|f| f.path(&self.db).starts_with(&pkg_root))
                .copied()
                .collect();
            pkg_files.sort_by_key(|f| f.path(&self.db).to_string_lossy().into_owned());

            let mut typed_asts: Vec<ast::typed::TypedFileAst> = Vec::new();
            let mut target_idx: Option<usize> = None;

            for pf in &pkg_files {
                let output = file_parse_output(&self.db, *pf);
                let parse_ast = ast::typed::ParseAst::from_file_ast(output.ast.clone());
                let file_id = ast::typed::FileId(typed_asts.len() as u32);
                let ctx = ast::typed::TransformCtx::from_typed_asts(&typed_asts);
                let typed = ast::typed::transform(parse_ast, file_id, &ctx);

                if pf.path(&self.db) == file_path {
                    target_idx = Some(typed_asts.len());
                }
                typed_asts.push(typed);
            }

            if let Some(idx) = target_idx {
                let typed = &typed_asts[idx];
                let source = file.contents(&self.db);
                let (line, character) = ast::byte_offset_to_position(byte_pos as usize, source);
                let hover_text = typed.hover_at(source, line, character)?;
                let source_name = file_path.parent().and_then(package_source_name);
                return match source_name {
                    Some(name) => Some(format!("`{name}`\n\n{hover_text}")),
                    None => Some(hover_text),
                };
            }
        }

        // Fallback: Salsa-tracked hover_markdown (no cross-file context).
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

/// Walk up from `dir` looking for a `flask.jsonc` package configuration.
fn find_package_root(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let (_config, root_dir) = flask::FlaskConfig::find_package_config(dir)?;
    Some(root_dir)
}

/// Compute the package-qualified source name (e.g. "core" or "core.arch") for
/// a file in a package. Returns `None` when no package config is found.
fn package_source_name(dir: &std::path::Path) -> Option<String> {
    let (config, root_dir) = flask::FlaskConfig::find_package_config(dir)?;
    if let Ok(rel) = dir.strip_prefix(&root_dir)
        && let Some(rel_str) = rel.to_str()
        && !rel_str.is_empty()
    {
        let subpath = rel_str.replace('/', ".");
        return Some(format!("{}.{subpath}", config.name));
    }
    Some(config.name)
}
