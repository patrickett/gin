//! Package-wide analysis: shared type environment across all parsed files in a package.

use ast::FileAst;
use database::{Db, File};
use diagnostic::Symptom;
use typeck;

/// Stable ordering for package [`File`] lists (path string).
pub fn sorted_package_files(db: &dyn Db, files: &[File]) -> Vec<File> {
    let mut v: Vec<File> = files.to_vec();
    v.sort_by_key(|f| f.path(db).to_string_lossy().into_owned());
    v
}

/// Intern sorted package files for [`queries::package_typecheck_symptoms`](crate::queries::package_typecheck_symptoms).
pub fn intern_package_files<'db>(db: &'db dyn Db, files: Vec<File>) -> crate::queries::PackageFiles<'db> {
    crate::queries::PackageFiles::new(db, files)
}

/// Run type-check and flow analysis for each file, using a shared [`typeck::TyEnv`]
/// built from all ASTs (same behavior as `begin build` / LSP diagnostics).
pub fn typecheck_symptoms_for_package(all_asts: &[FileAst]) -> Vec<Vec<Symptom>> {
    (0..all_asts.len())
        .map(|i| typeck::analyze_file(&all_asts[i], all_asts))
        .collect()
}
