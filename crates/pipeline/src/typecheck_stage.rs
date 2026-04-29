use typeck::TyEnv;

use crate::{ParsedFile, TypecheckResult};

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
