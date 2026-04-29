pub mod resolve;
pub use resolve::resolve_imports;

mod module_graph;

use diagnostic::{Category, Diagnostic};
use parser::parse_source_full;
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

/// Parse source texts into ASTs.
///
/// Each `(PathBuf, String)` pair is a file path and its contents.
/// Parse diagnostics are stored in each `ParsedFile`'s `output.symptoms`.
pub fn parse(sources: &[(PathBuf, String)]) -> Vec<ParsedFile> {
    sources
        .iter()
        .map(|(path, source)| {
            let output = parse_source_full(source);
            ParsedFile {
                path: path.clone(),
                source: source.clone(),
                output,
            }
        })
        .collect()
}

/// Type-check a package of parsed files.
///
/// Builds a shared `TyEnv` from all file ASTs, then runs per-file analysis
/// against it. Results are returned in `TypecheckResult::symptoms`, parallel
/// to the input slice.
pub fn typecheck(files: &[ParsedFile]) -> TypecheckResult {
    let asts: Vec<_> = files.iter().map(|f| f.output.ast.clone()).collect();
    let ty_env = TyEnv::from_multiple_file_asts(&asts);

    let symptoms = asts
        .iter()
        .map(|ast| typeck::analyze_file_with_ty_env(ast, &ty_env))
        .collect();

    TypecheckResult { ty_env, symptoms }
}

/// Returns `true` if any file contains a fatal (`Flaw`) diagnostic.
pub fn has_fatal(files: &[ParsedFile]) -> bool {
    files
        .iter()
        .any(|f| f.output.symptoms.iter().any(|d| matches!(d.category, Category::Flaw)))
}
