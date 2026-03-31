//! Analysis utilities for type environments and file analysis.

mod pipeline;
mod ty_env;

pub use pipeline::{analyze_file, analyze_package};
pub use ty_env::ty_env_for_file;
