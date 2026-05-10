#![deny(unsafe_code)]
#![warn(
    clippy::correctness,
    clippy::suspicious,
    clippy::style,
    clippy::complexity,
    clippy::perf
)]
pub mod package_resolver;
pub mod import_query;
pub mod file_helpers;

mod module_graph;

// Re-export batch pipeline
pub use package_resolver::{
    ResolveGraph, ResolveNode, merge_asts_checked, resolve_import_symptoms, resolve_imports,
};

// Re-export per-request queries
pub use import_query::{
    ImportTarget, default_file_reader, part_index_in_dotted_path, resolve_dep_dir,
    resolve_dep_hover, resolve_import_at, resolve_symbol_def_span, resolve_symbol_hover,
};

// Re-export file helpers
pub use file_helpers::{
    check_public_def_in_package, collect_gin_files, collect_gin_files_recursive,
    find_public_def_in_package, is_folder_module_dir, list_public_symbols,
    resolve_flask_path_dependencies,
};

use parser::ParseOutput;
use std::path::PathBuf;

#[derive(Clone)]
pub struct ParsedFile {
    pub path: PathBuf,
    pub source: String,
    pub output: ParseOutput,
}

impl ParsedFile {
    pub fn filename(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}
