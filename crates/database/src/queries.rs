use crate::{Db, File, InputDatabase};
use ast::FileAst;
use parser::ParseOutput;
use std::sync::Arc;

/// Full parse (lexer + parser + import checks) cached per [`File`].
///
/// Use this when diagnostics or hover need the same AST and [`span::SpanTable`]
/// as `parse_source_full`.
#[salsa::tracked]
pub fn file_parse_output(db: &dyn Db, file: File) -> Arc<ParseOutput> {
    let source = file.contents(db);
    Arc::new(parser::parse_source_full(source))
}

/// Parse a file using Salsa so repeated requests are cached and automatically
/// invalidated when the file contents change.
///
/// AST-only, same as [`file_parse_output`]'s tree (via `parse_source_full`), without
/// retaining span tables / parse symptoms in the return type.
#[salsa::tracked]
pub fn parse_file(db: &dyn Db, file: File) -> Arc<FileAst> {
    Arc::new(file_parse_output(db, file).ast.clone())
}

/// Update a file's contents without requiring downstream crates to depend on Salsa.
pub fn set_file_contents(db: &mut InputDatabase, file: File, contents: String) {
    use salsa::Setter;
    file.set_contents(db).to(contents);
}
