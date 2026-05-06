#![deny(unsafe_code)]
#![warn(clippy::correctness, clippy::suspicious, clippy::style, clippy::complexity, clippy::perf)]
//! Parser for the Gin language.
//!
//! This crate contains the recursive-descent parser and content hashing.
//! All public functions are pure — no Salsa or database dependency is required.

pub mod content_hash;
mod cursor;
pub mod declare;
pub mod expr;
mod impl_block;
pub mod module;
pub mod params;
pub mod path;
pub mod query;
pub mod tag;
mod top_level;
pub mod unescape;

pub use cursor::ParseError;
pub use unescape::*;

// Re-export convenience functions
pub use expr::parse_source as parse_from_str;

// Re-export full parsing API
pub use query::{
    ParseOutput, extract_local_import_paths, extract_package_import_paths, parse_source_full,
};

// Re-export module discovery
pub use module::{ModuleTree, discover_module, discover_module_at};
