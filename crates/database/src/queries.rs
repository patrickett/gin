use crate::{Db, File, InputDatabase};
use ast::FileAst;
use std::sync::Arc;

/// Parse a file using Salsa so repeated requests are cached and automatically
/// invalidated when the file contents change.
#[salsa::tracked]
pub fn parse_file(db: &dyn Db, file: File) -> Arc<FileAst> {
    let source = file.contents(db);
    Arc::new(parser::parse_from_str(source))
}

/// Update a file's contents without requiring downstream crates to depend on Salsa.
pub fn set_file_contents(db: &mut InputDatabase, file: File, contents: String) {
    use salsa::Setter;
    file.set_contents(db).to(contents);
}

