//! Persistent inputs and low-level Salsa queries (for example parsing).
//! Semantic analysis and editor integration live in the `analyze` and `ide` crates.
pub mod input_database;
pub mod queries;

pub use input_database::{Db, InputDatabase};
pub use queries::{file_parse_output, parse_file, set_file_contents};

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

/// Salsa accumulator for diagnostics.
///
/// Defined here (rather than in `diagnostic`) so that the `diagnostic` crate
/// doesn't need to depend on `salsa`. Use [`EmitDiagnostic::emit`] at call sites
/// and `accumulated::<Diagnostics>` to retrieve accumulated diagnostics.
#[salsa::accumulator]
#[derive(Debug, Clone)]
pub struct Diagnostics(pub Diagnostic);

impl Deref for Diagnostics {
    type Target = Diagnostic;

    fn deref(&self) -> &Diagnostic {
        &self.0
    }
}

/// Extension trait that provides the `.emit()` convenience method for
/// accumulating diagnostics into a Salsa database.
///
/// Import this trait wherever you need to emit diagnostics:
/// ```ignore
/// use database::EmitDiagnostic;
/// SomeSymptom.emit(db, span_id);
/// ```
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
