pub mod input_database;

use std::path::PathBuf;

#[salsa::input]
pub struct File {
    pub path: PathBuf,
    #[returns(ref)]
    pub contents: String,
}

/// Compiled module containing bytecode and metadata.
#[salsa::tracked]
pub struct CompiledModule<'db> {
    #[returns(ref)]
    pub bytecode: Vec<u8>,
}
