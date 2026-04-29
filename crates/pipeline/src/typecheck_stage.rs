use ast::FileAst;
use diagnostic::Category;
use typeck::{TyEnv, analyze_file};

use crate::{ResolveResult, TypecheckResult};

/// Type-check a resolved package.
///
/// Builds a shared `TyEnv` from all file ASTs, then runs per-file analysis
/// to collect type-check and flow-analysis diagnostics. Typecheck diagnostics
/// are appended to each file's `output.symptoms` so they can be printed with
/// the correct span table.
pub fn typecheck(resolved: &ResolveResult) -> TypecheckResult {
    let asts: Vec<FileAst> = resolved.files.iter().map(|f| f.output.ast.clone()).collect();
    let ty_env = TyEnv::from_multiple_file_asts(&asts);

    let mut files = resolved.files.clone();
    let mut all_diagnostics = resolved.diagnostics.clone();

    for (i, file) in files.iter_mut().enumerate() {
        let symptoms = analyze_file(&asts[i], &asts);
        all_diagnostics.extend(symptoms.clone());
        file.output.symptoms.extend(symptoms);
    }

    TypecheckResult {
        files,
        ty_env,
        diagnostics: all_diagnostics,
    }
}

/// Print all diagnostics for a slice of parsed files.
///
/// Each file's symptoms are printed with its own span table and source text.
/// Returns `true` if any fatal diagnostics were found.
pub fn print_diagnostics(files: &[crate::ParsedFile]) -> bool {
    let mut has_flaws = false;
    for file in files {
        let filename = file.filename();
        let span_table = file.output.ast.span_table();
        for diag in &file.output.symptoms {
            diag.print(span_table, &file.source, &filename);
            if matches!(diag.category, Category::Flaw) {
                has_flaws = true;
            }
        }
    }
    has_flaws
}
