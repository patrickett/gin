pub mod input_database;
pub mod queries;

pub use input_database::{Db, InputDatabase};
pub use queries::{parse_file, set_file_contents};

use diagnostic::{Symptom, SymptomLike};
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
/// doesn't need to depend on `salsa`. Use [`EmitSymptom::emit`] at call sites
/// and `accumulated::<Symptoms>` to retrieve accumulated diagnostics.
#[salsa::accumulator]
#[derive(Debug, Clone)]
pub struct Symptoms(pub Symptom);

impl Deref for Symptoms {
    type Target = Symptom;

    fn deref(&self) -> &Symptom {
        &self.0
    }
}

/// Extension trait that provides the `.emit()` convenience method for
/// accumulating diagnostics into a Salsa database.
///
/// Import this trait wherever you need to emit diagnostics:
/// ```ignore
/// use database::EmitSymptom;
/// SomeSymptom.emit(db, span_id);
/// ```
pub trait EmitSymptom: SymptomLike {
    fn emit<D: salsa::Database + ?Sized>(self, db: &D, span_id: SpanId) {
        use salsa::Accumulator;
        Symptoms(self.into_symptom(span_id)).accumulate(db);
    }
}

impl<T: SymptomLike> EmitSymptom for T {}
