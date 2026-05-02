pub mod resolve;
pub use resolve::{merge_asts_checked, resolve_imports, resolve_flask_path_dependencies};

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
