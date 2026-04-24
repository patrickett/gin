use database::{Db, File};

/// Compute hover markdown via Salsa, keyed by `(file, byte_pos)`.
///
/// This is a pure function of the file contents + cursor position, so Salsa
/// caching is a natural fit.
#[salsa::tracked]
pub fn hover_markdown(db: &dyn Db, file: File, byte_pos: u32) -> Option<String> {
    let source = file.contents(db);
    let ast = database::parse_file(db, file);
    crate::hover_at(source, &ast, byte_pos as usize)
}

