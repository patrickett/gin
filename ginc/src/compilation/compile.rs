//! Analysis pipeline — single entry point for parsing, type checking, and flow analysis.

use crate::ast::FileAst;
use crate::database::{File, input_database::Db};
use crate::diagnostic::type_ as type_symptom;
use crate::parse::parse;
use crate::parse::resolve_imports;
use crate::typeck::{TyEnv, flow_analyzer::FlowAnalyzer};
use salsa::Accumulator;

/// Build a shared type environment for a file and all its transitive imports.
///
/// This is a lightweight helper for callers (e.g. LSP hover) that need a `TyEnv`
/// without running the full analysis pipeline.
pub fn ty_env_for_file(db: &dyn Db, file: File) -> TyEnv {
    let imported_files = resolve_imports(db, file);
    let mut all_files: Vec<File> = vec![file];
    all_files.extend(imported_files);
    let all_asts: Vec<FileAst> = all_files.iter().map(|&f| parse(db, f)).collect();
    TyEnv::from_multiple_file_asts(&all_asts)
}

/// Analyze a single file within a package context.
///
/// This per-file tracked function performs type checking and flow analysis
/// for one file, using a shared type environment built from all files in the
/// package. This ensures cross-file type visibility (single-compilation-unit
/// semantics).
///
/// All diagnostics are accumulated via Salsa and can be retrieved with:
/// ```ignore
/// analyze_file::accumulated::<Symptom>(db, file, all_files)
/// ```
///
/// # Arguments
/// * `file`      — The file to analyze
/// * `all_files` — All files in the package (for building the shared type environment)
#[salsa::tracked]
pub fn analyze_file<'db>(db: &'db dyn Db, file: File, all_files: Vec<File>) -> FileAst {
    // Build shared TyEnv from all package files so cross-file types are visible.
    // The per-file `parse` calls inside are cached by Salsa.
    let all_asts: Vec<FileAst> = all_files.iter().map(|&f| parse(db, f)).collect();
    let ty_env = TyEnv::from_multiple_file_asts(&all_asts);

    // Parse this file (cached by Salsa)
    let ast = parse(db, file);

    // Type check — emits unknown type / binding / variable symptoms
    ty_env.check_unknowns(&ast, db);

    // Flow analysis — emits bounds-check symptoms
    let mut analyzer = FlowAnalyzer::new(&ty_env);
    analyzer.analyze_file(&ast);
    let result = analyzer.into_result();

    for check in &result.bounds_checks {
        type_symptom::index_out_of_bounds(check.span, check.index, check.size).accumulate(db);
    }

    ast
}

/// Analyze a package of Gin source files.
///
/// This is the single entry point for the analysis phase of the compilation
/// pipeline. It delegates to [`analyze_file`] for each file, giving each call
/// the full file list so that a shared type environment is used throughout.
///
/// Returns the parsed ASTs for downstream codegen.
///
/// # Usage
///
/// - **Binary compilation**: pass the entry file + all resolved imports
/// - **Library compilation**: pass all `.gin` files in the library directory
/// - **LSP diagnostics**: call [`analyze_file`] directly for single-file queries
///
/// **Note:** This is a regular (non-tracked) function. It doesn't need to be
/// tracked by Salsa because it just orchestrates calls to the tracked
/// [`analyze_file`] function, which handles its own caching and diagnostic
/// accumulation per file.
pub fn analyze_package(db: &dyn Db, files: Vec<File>) -> Vec<FileAst> {
    let all_files = files.clone();

    files
        .iter()
        .map(|&file| analyze_file(db, file, all_files.clone()))
        .collect()
}
