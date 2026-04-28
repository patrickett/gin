//! Salsa-tracked semantic queries (type environments, diagnostics, hover).
//!
//! Migrated from the `analyze` crate. These queries wrap `typeck` functions
//! with Salsa caching for IDE use.

use crate::{Db, File};
use crate::package::PackageFiles;
use diagnostic::Diagnostic;
use std::sync::Arc;

/// Compute hover markdown via Salsa, keyed by `(file, byte_pos)`.
///
/// This is a pure function of the file contents + cursor position, so Salsa
/// caching is a natural fit. Parsing matches [`crate::file_parse_output`]
/// (full lexer + parse diagnostics path).
#[salsa::tracked]
pub fn hover_markdown(db: &dyn Db, file: File, byte_pos: u32) -> Option<String> {
    let source = file.contents(db);
    let output = crate::file_parse_output(db, file);
    typeck::hover_at(source, &output.ast, byte_pos as usize)
}

/// Full parse output for `file` (shared with diagnostics / hover).
pub fn file_parse_output(db: &dyn Db, file: File) -> Arc<parser::ParseOutput> {
    crate::file_parse_output(db, file)
}

/// Package-wide [`typeck::TyEnv`] (shared across all [`File`]s in `pkg`), Salsa-cached.
///
/// Build `pkg` with [`PackageFiles::new`](PackageFiles::new) after
/// [`crate::package::sorted_package_files`].
#[salsa::tracked]
pub fn package_ty_env<'db>(db: &'db dyn Db, pkg: PackageFiles<'db>) -> Arc<typeck::TyEnv> {
    let files = pkg.files(db);
    let all_asts: Vec<ast::FileAst> = files
        .iter()
        .map(|&f| crate::file_parse_output(db, f).ast.clone())
        .collect();
    Arc::new(typeck::TyEnv::from_multiple_file_asts(&all_asts))
}

/// Type-check + flow-analysis symptoms for every file in the package, sharing one
/// [`typeck::TyEnv`] built from all parsed ASTs (Salsa-cached).
///
/// Uses [`package_ty_env`] so the environment is memoized independently of per-file checks.
#[salsa::tracked]
pub fn package_typecheck_symptoms<'db>(
    db: &'db dyn Db,
    pkg: PackageFiles<'db>,
) -> Vec<Vec<Diagnostic>> {
    let ty_env = package_ty_env(db, pkg);
    let files = pkg.files(db);
    files
        .iter()
        .map(|&f| {
            let ast = &crate::file_parse_output(db, f).ast;
            typeck::analyze_file_with_ty_env(ast, &ty_env)
        })
        .collect()
}

/// Type environment for a single file (from that file's AST only).
///
/// For a package-wide env, use [`package_ty_env`].
pub fn ty_env_for_file(db: &dyn Db, file: File) -> Arc<typeck::TyEnv> {
    let ast = &crate::file_parse_output(db, file).ast;
    Arc::new(typeck::TyEnv::from_file_ast(ast))
}
