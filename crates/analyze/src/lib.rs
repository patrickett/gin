//! Incremental analysis ("engine") for Gin.
//!
//! This crate hosts Salsa-tracked queries and semantic functionality used by IDE tooling.

pub mod completions;
pub mod hover;
pub mod package;
pub mod queries;

pub use completions::*;
pub use hover::{dot_type_at, find_definition_span, find_references, hover_at};
pub use package::*;
pub use queries::{
    file_parse_output, hover_markdown, package_ty_env, package_typecheck_symptoms, ty_env_for_file,
    PackageFiles,
};

