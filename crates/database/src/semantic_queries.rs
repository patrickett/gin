//! Salsa-tracked semantic queries (type environments, diagnostics, hover).
//!
//! Migrated from the `analyze` crate. These queries wrap `typeck` functions
//! with Salsa caching for IDE use.

use crate::package::PackageFiles;
use crate::{Db, File};
use diagnostic::Diagnostic;
use std::sync::Arc;

/// This is a pure function of the file contents + cursor position, so Salsa
/// caching is a natural fit. Parsing matches [`crate::file_parse_output`]
/// (full lexer + parse diagnostics path).
///
/// Uses the new [`TypedFileAst`](ast::typed::TypedFileAst) API for hover info.
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

    // Convert byte position to line/character for the typed AST hover API.
    let (line, character) = ast::byte_offset_to_position(byte_pos as usize, source);

    // Build the typed AST using the new transformation pipeline.
    let typed = ast::typed::transform_file(output.ast.clone(), ast::typed::FileId(0));

    let hover_text = typed.hover_at(source, line, character)?;

    // Prefix with module path if available (mirrors old hover_at_with_source behavior).
    match source_name {
        Some(name) => Some(format!("`{name}`\n\n{hover_text}")),
        None => Some(hover_text),
    }
}

pub fn file_parse_output(db: &dyn Db, file: File) -> Arc<parser::ParseOutput> {
    crate::file_parse_output(db, file)
}

/// Shares one type environment built from all parsed ASTs across files.
///
/// Uses the new [`TypedFileAst`](ast::typed::TypedFileAst) API. Each file is
/// transformed with access to previously-transformed files via
/// [`TransformCtx::from_typed_asts`](ast::typed::TransformCtx::from_typed_asts).
/// Type symptoms are collected and converted to `Diagnostic` for the existing API.
#[salsa::tracked]
pub fn package_typecheck_symptoms<'db>(
    db: &'db dyn Db,
    pkg: PackageFiles<'db>,
) -> Vec<Vec<Diagnostic>> {
    let files = pkg.files(db);
    let mut typed_asts: Vec<ast::typed::TypedFileAst> = Vec::new();
    let mut results: Vec<Vec<Diagnostic>> = Vec::new();

    for file in files.iter() {
        let output = crate::file_parse_output(db, *file);
        let parse_ast = ast::typed::ParseAst::from_file_ast(output.ast.clone());
        let file_id = ast::typed::FileId(typed_asts.len() as u32);

        // Build cross-file context from already-transformed files.
        let ctx = ast::typed::TransformCtx::from_typed_asts(&typed_asts);
        let typed = ast::typed::transform(parse_ast, file_id, &ctx);

        // Collect flaws and convert to Diagnostic.
        let mut file_diags: Vec<Diagnostic> = Vec::new();
        for (expr_id, flaw) in typed.all_flaws() {
            use diagnostic::DiagnosticLike;
            let span_id = typed.exprs.span[expr_id.as_usize()];
            file_diags.push(flaw.clone().into_diagnostic(span_id));
        }

        results.push(file_diags);
        typed_asts.push(typed);
    }

    results
}
