//! Hover functionality (thin wrapper).
//!
//! Semantic hover analysis lives in `crates/analyze`.

pub use analyze::{dot_type_at, find_definition_span, find_references, hover_at};
