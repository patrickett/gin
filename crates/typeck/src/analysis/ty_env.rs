//! Type environment construction for files.

use ast::FileAst;
use ast::{parse_file, resolve_imports};
use database::{File, Db};
use crate::TyEnv;

/// Build a shared type environment for a file and all its transitive imports.
///
/// This is a lightweight helper for callers (e.g. LSP hover) that need a `TyEnv`
/// without running the full analysis pipeline.
pub fn ty_env_for_file(db: &dyn Db, file: File) -> TyEnv {
    let imported_files = resolve_imports(db, file);
    let mut all_files: Vec<File> = vec![file];
    all_files.extend(imported_files);
    let all_asts: Vec<FileAst> = all_files.iter().map(|&f| parse_file(db, f)).collect();
    TyEnv::from_multiple_file_asts(&all_asts)
}
