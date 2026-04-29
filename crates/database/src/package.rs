//! Package-wide analysis: shared type environment across all parsed files in a package.

use crate::{Db, File};

/// Interned package file set (stable identity for Salsa tracked queries).
#[salsa::interned]
pub struct PackageFiles<'db> {
    #[returns(ref)]
    pub files: Vec<File>,
}

/// Stable ordering for package [`File`] lists (path string).
pub fn sorted_package_files(db: &dyn Db, files: &[File]) -> Vec<File> {
    let mut v: Vec<File> = files.to_vec();
    v.sort_by_key(|f| f.path(db).to_string_lossy().into_owned());
    v
}

/// Intern sorted package files for [`super::semantic_queries::package_typecheck_symptoms`].
pub fn intern_package_files<'db>(db: &'db dyn Db, files: Vec<File>) -> PackageFiles<'db> {
    PackageFiles::new(db, files)
}
