#![deny(unsafe_code)]
#![warn(
    clippy::correctness,
    clippy::suspicious,
    clippy::style,
    clippy::complexity,
    clippy::perf
)]

pub mod engine;
pub mod salsa_engine;

// Internal Salsa infrastructure — modules are pub for macro codegen but
// their types should NOT be imported by external crates. Use QueryEngine instead.
pub mod input_database;
pub mod package;
pub mod queries;
pub mod semantic_queries;

// Re-export only the QueryEngine seam.
pub use engine::QueryEngine;
pub use salsa_engine::SalsaQueryEngine;

// ---------------------------------------------------------------------------
// Salsa types (kept for backward compat during migration; new code should use
// QueryEngine instead).
// ---------------------------------------------------------------------------
pub use input_database::{Db, InputDatabase};
pub use package::{PackageFiles, intern_package_files, sorted_package_files};
pub use queries::{file_parse_output, parse_file, set_file_contents};
pub use semantic_queries::{hover_markdown, package_typecheck_symptoms};

use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticLike};
use span::SpanId;
use std::ops::Deref;
use std::path::PathBuf;

#[salsa::input]
pub struct File {
    pub path: PathBuf,
    #[returns(ref)]
    pub contents: String,
}

#[salsa::accumulator]
#[derive(Debug, Clone)]
pub struct Diagnostics(pub Diagnostic);

impl Deref for Diagnostics {
    type Target = Diagnostic;

    fn deref(&self) -> &Diagnostic {
        &self.0
    }
}

pub trait EmitDiagnostic: DiagnosticLike {
    fn emit<D: salsa::Database + ?Sized>(self, db: &D, span_id: SpanId)
    where
        Self: Into<DiagnosticCode>,
    {
        use salsa::Accumulator;
        Diagnostics(self.into_diagnostic(span_id)).accumulate(db);
    }
}

impl<T: DiagnosticLike + Into<DiagnosticCode>> EmitDiagnostic for T {}
