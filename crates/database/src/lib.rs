pub mod input_database;

pub use input_database::{Db, InputDatabase};

use std::path::PathBuf;

#[salsa::input]
pub struct File {
    pub path: PathBuf,
    #[returns(ref)]
    pub contents: String,
}
