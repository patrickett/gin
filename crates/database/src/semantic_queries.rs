//! Salsa-tracked semantic queries (type environments, diagnostics, hover).
//!
//! Migrated from the `analyze` crate. These queries wrap `typeck` functions
//! with Salsa caching for IDE use.

use crate::package::PackageFiles;
use crate::{Db, File};
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
    let source_name = file.path(db).parent().and_then(|dir| {
        // Walk up to find the flask.jsonc package root and compute the full module path
        // (e.g., "core.arch" for modules/gin_core/arch/x86_64.gin when gin_core is named "core").
        if let Some((config, root_dir)) = flask::FlaskConfig::find_package_config(dir) {
            // Compute relative path from package root to the file's parent dir
            if let Ok(rel) = dir.strip_prefix(&root_dir)
                && let Some(rel_str) = rel.to_str()
                && !rel_str.is_empty()
            {
                let subpath = rel_str.replace('/', ".");
                return Some(format!("{}.{subpath}", config.name));
            }
            return Some(config.name);
        }
        // Fall back to the directory name.
        dir.file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_owned())
    });
    let analysis = ast::resolve_types(&output.ast, std::slice::from_ref(&output.ast));
    match source_name.as_deref() {
        Some(name) => {
            ast::hover::hover_at_with_source(source, &output.ast, byte_pos as usize, Some(name))
        }
        None => ast::hover::hover_at(source, &output.ast, &analysis, byte_pos as usize),
    }
}

/// Full parse output for `file` (shared with diagnostics / hover).
pub fn file_parse_output(db: &dyn Db, file: File) -> Arc<parser::ParseOutput> {
    crate::file_parse_output(db, file)
}

/// Type-check + flow-analysis symptoms for every file in the package, sharing one
/// type environment built from all parsed ASTs.
///
/// Diagnostics are embedded in each resolved FileAst via [`ast::FileAst::resolve_types`].
#[salsa::tracked]
pub fn package_typecheck_symptoms<'db>(
    db: &'db dyn Db,
    pkg: PackageFiles<'db>,
) -> Vec<Vec<Diagnostic>> {
    let files = pkg.files(db);
    let all_asts: Vec<ast::FileAst> = files
        .iter()
        .map(|&f| crate::file_parse_output(db, f).ast.clone())
        .collect();
    files
        .iter()
        .map(|&f| {
            let mut ast = crate::file_parse_output(db, f).ast.clone();
            let analysis = ast::resolve_types(&ast, &all_asts);
            ast::populate_ast_types(&mut ast, &analysis);
            analysis.diagnostics
        })
        .collect()
}
