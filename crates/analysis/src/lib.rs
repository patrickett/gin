pub mod cursor;
pub mod engine;
pub mod hover;
pub mod package_cache;

// Internal modules used by PackageCache.
mod semantic_queries;
pub use semantic_queries::HoverDocExt;

// Public IDE / compiler analysis API.
pub use cursor::ReferencesInFile;
pub use engine::{DocumentSnapshot, FileSemanticOutput, HoverContent};
pub use package_cache::PackageCache;
