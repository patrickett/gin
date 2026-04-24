//! Incremental analysis ("engine") for Gin.
//!
//! This crate hosts Salsa-tracked queries and semantic functionality used by IDE tooling.

pub mod hover;
pub mod queries;

pub use hover::*;
pub use queries::*;

