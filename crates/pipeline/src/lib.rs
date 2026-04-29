pub mod parse_stage;
pub use parse_stage::parse;

pub mod resolve;
pub use resolve::resolve_imports;

pub mod typecheck_stage;
pub use typecheck_stage::typecheck;

mod module_graph;

use diagnostic::{Category, Diagnostic};
use parser::ParseOutput;
use std::path::PathBuf;
use typeck::TyEnv;

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

pub struct TypecheckResult {
    pub ty_env: TyEnv,
    /// Per-file type-check and flow-analysis diagnostics, parallel to the
    /// input `ParsedFile` slice.
    pub symptoms: Vec<Vec<Diagnostic>>,
}

/// Returns `true` if any file contains a fatal (`Flaw`) diagnostic.
pub fn has_fatal(files: &[ParsedFile]) -> bool {
    files
        .iter()
        .any(|f| f.output.symptoms.iter().any(|d| matches!(d.category, Category::Flaw)))
}
