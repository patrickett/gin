pub mod cursor;
pub mod engine;
pub mod hover;
pub mod package_cache;
mod queries;

#[cfg(test)]
extern crate self as analysis;

// Internal modules used by PackageCache.
mod semantic_queries;
pub use semantic_queries::format_hover_markdown;

// Public IDE / compiler analysis API.
pub use cursor::ReferencesInFile;
pub use engine::{DocumentSnapshot, FileSemanticOutput, HoverContent};
pub use package_cache::PackageCache;
pub use queries::{ScopeSymbol, SemanticQueries, SymbolKind};
