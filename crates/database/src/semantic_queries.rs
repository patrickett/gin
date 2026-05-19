//! Salsa-tracked semantic queries (type environments, diagnostics, hover).
//!
//! Migrated from the `analyze` crate. These queries wrap `typeck` functions
//! with Salsa caching for IDE use.

use crate::package::PackageFiles;
use crate::{Db, File};
use diagnostic::Diagnostic;
use std::sync::Arc;

/// Replace `gin:marker/Name` links with `file://` URIs to the actual marker definition files.
fn resolve_marker_links(hover_text: &str, core_root: &std::path::Path) -> String {
    let mut result = hover_text.to_string();
    // Pattern: `gin:marker/Name` where Name is a capitalised marker type
    while let Some(start) = result.find("gin:marker/") {
        let after_prefix = &result[start + 11..];
        let end = after_prefix
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(after_prefix.len());
        let marker_name = &after_prefix[..end];
        let file_path = core_root
            .join("marker")
            .join(format!("{}.gin", marker_name.to_lowercase()));
        // Build a file:// URI manually (macOS/Linux paths)
        let path_str = file_path.to_string_lossy();
        // Ensure the path starts with /
        let uri = if path_str.starts_with('/') {
            format!("file://{}", path_str)
        } else {
            format!("file:///{}", path_str)
        };
        result.replace_range(start..start + 11 + end, &uri);
    }
    result
}

/// Find the gin_core package root directory by searching for a `flask.jsonc`
/// whose package name is "core". This is where marker definitions live.
fn find_core_package_root() -> Option<std::path::PathBuf> {
    // Search upward from the current working directory or use a known path
    let candidates = [
        std::env::current_dir().ok(),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf())),
    ];
    for start in candidates.into_iter().flatten() {
        let mut dir = start.clone();
        loop {
            if let Some((config, root_dir)) = flask::FlaskConfig::find_package_config(&dir)
                && config.name == "core"
            {
                return Some(root_dir);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    None
}

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

    // Resolve gin:marker/Name links to file:// URIs pointing to the marker's
    // gin_core definition (e.g. `gin:marker/Copy` → `file:///path/to/.../marker/copy.gin`).
    let hover_text = if let Some(core_root) = find_core_package_root() {
        resolve_marker_links(&hover_text, &core_root)
    } else {
        hover_text
    };

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
