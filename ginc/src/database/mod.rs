pub mod input_database;
pub mod interface_hash;

pub use interface_hash::file_interface_hash;

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
