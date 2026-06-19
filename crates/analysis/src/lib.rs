pub mod cache_engine;
pub mod cursor;
pub mod engine;
pub mod hover;
pub mod package_cache;

// Internal modules used by PackageCache and CacheEngine.
mod semantic_queries;
pub use semantic_queries::HoverDocExt;

// Public IDE / compiler query seam.
pub use cache_engine::CacheEngine;
pub use cursor::ReferencesInFile;
pub use engine::{
    DocumentSnapshot, FileSemanticOutput, FileTypedOutput, HoverContent, QueryEngine,
};
pub use package_cache::{PackageCache, ResolveAndPrepareExt};
