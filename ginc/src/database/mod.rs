pub mod accumulator;
pub mod input_database;
pub mod queries;

pub use queries::{ast_hash, compile, compile_entry, parse, parse_dependencies};
use std::path::PathBuf;

#[salsa::input]
pub struct File {
    path: PathBuf,
    #[returns(ref)]
    contents: String,
}

/// A file dependency (import).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FileDependency {
    pub file: File,
    pub import_path: String,
}

/// Compiled module containing bytecode and metadata.
#[salsa::tracked]
pub struct CompiledModule<'db> {
    #[returns(ref)]
    pub bytecode: Vec<u8>,
}
