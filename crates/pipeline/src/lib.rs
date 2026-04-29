pub mod collect;
pub use collect::collect;

pub mod parse_stage;
pub use parse_stage::parse;

pub mod resolve;
pub use resolve::resolve_imports;

pub mod typecheck_stage;
pub use typecheck_stage::typecheck;
pub use typecheck_stage::print_diagnostics;

mod module_graph;

use diagnostic::{Category, Diagnostic};
use parser::ParseOutput;
use std::path::PathBuf;
use typeck::TyEnv;

pub struct SourceCollection {
    pub file_paths: Vec<PathBuf>,
    pub is_library: bool,
}

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

pub struct ParseResult {
    pub files: Vec<ParsedFile>,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParseResult {
    pub fn has_fatal(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| matches!(d.category, Category::Flaw))
    }
}

pub struct ResolveResult {
    pub files: Vec<ParsedFile>,
    pub diagnostics: Vec<Diagnostic>,
}

impl ResolveResult {
    pub fn has_fatal(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| matches!(d.category, Category::Flaw))
    }
}

pub struct TypecheckResult {
    pub files: Vec<ParsedFile>,
    pub ty_env: TyEnv,
    pub diagnostics: Vec<Diagnostic>,
}

impl TypecheckResult {
    pub fn has_fatal(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| matches!(d.category, Category::Flaw))
    }
}
