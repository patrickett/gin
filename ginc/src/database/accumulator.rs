use salsa::Accumulator;

use crate::database::{input_database::Db, File};

#[salsa::accumulator]
pub struct Diagnostic(pub String);

impl Diagnostic {
    // error: Report
    pub fn push_error(db: &dyn Db, file: File, error: String) {
        Diagnostic(format!(
            "Error in file {}: {:?}\n",
            file.path(db)
                .file_name()
                .unwrap_or_else(|| "<unknown>".as_ref())
                .to_string_lossy(),
            error
        ))
        .accumulate(db);
    }
}
