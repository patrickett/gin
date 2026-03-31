//! Interface hash computation as a Salsa tracked query.

use crate::compute_interface_hash;
use crate::parse::query::parse;
use database::{File, Db};

/// File interface hash computed from AST.
///
/// This is a Salsa tracked struct that wraps a hash string,
/// allowing the result to be cached and only recomputed when
/// signatures change.
#[salsa::tracked]
pub struct FileInterfaceHash<'db> {
    #[returns(ref)]
    pub hash: String,
}

/// Compute the interface hash for a file.
///
/// This is a tracked query, so the result is cached and only recomputed
/// when the file's AST changes in ways that affect the interface (signatures).
#[salsa::tracked]
pub fn file_interface_hash<'db>(
    db: &'db dyn Db,
    file: File,
) -> FileInterfaceHash<'db> {
    let ast = parse(db, file);
    let hash = compute_interface_hash(&ast);
    FileInterfaceHash::new(db, hash)
}
