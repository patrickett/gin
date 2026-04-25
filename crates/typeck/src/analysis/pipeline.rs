//! Analysis pipeline — single entry point for type checking and flow analysis.

use crate::{FlowAnalyzer, TyEnv};
use ast::FileAst;
use diagnostic::SymptomLike;
use diagnostic::type_::TypeSymptom;

/// Analyze a single file's AST for type errors and flow issues.
///
/// Returns all collected symptoms (type errors, flow warnings, etc.).
/// The caller is responsible for parsing and providing the ASTs.
///
/// # Arguments
/// * `ast`       — The AST of the file to analyze
/// * `all_asts`  — All ASTs in the package (for building the shared type environment)
/// Type-check and flow-analyze `ast` using a pre-built package [`TyEnv`].
pub fn analyze_file_with_ty_env(ast: &FileAst, ty_env: &TyEnv) -> Vec<diagnostic::Symptom> {
    let mut symptoms = Vec::new();

    ty_env.check_unknowns(ast, &mut symptoms);

    let mut analyzer = FlowAnalyzer::new(ty_env);
    analyzer.analyze_file(ast);
    let result = analyzer.into_result();

    for check in &result.bounds_checks {
        symptoms.push(
            TypeSymptom::IndexOutOfBounds {
                index: check.index,
                size: check.size,
            }
            .into_symptom(check.span),
        );
    }

    symptoms
}

pub fn analyze_file(ast: &FileAst, all_asts: &[FileAst]) -> Vec<diagnostic::Symptom> {
    let ty_env = TyEnv::from_multiple_file_asts(all_asts);
    analyze_file_with_ty_env(ast, &ty_env)
}

/// Analyze a package of pre-parsed ASTs.
///
/// Returns all collected symptoms from all files.
pub fn analyze_package(asts: &[FileAst]) -> Vec<diagnostic::Symptom> {
    let mut all_symptoms = Vec::new();
    for ast in asts {
        all_symptoms.extend(analyze_file(ast, asts));
    }
    all_symptoms
}
