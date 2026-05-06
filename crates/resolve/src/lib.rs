pub mod resolve;
pub use resolve::{
    check_public_def_in_package, collect_gin_files, collect_gin_files_recursive,
    discovery, find_public_def_in_package, is_folder_module_dir, merge_asts_checked,
    resolve_dep_hover, resolve_import_symptoms, resolve_imports,
    part_index_in_dotted_path, resolve, resolve_dep_dir, resolve_flask_path_dependencies,
    resolve_import_at, resolve_symbol_def_span, resolve_symbol_hover,
    ImportTarget, ResolveGraph, ResolveNode,
};

mod module_graph;

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
