use crate::span::{HasSpanId, SpanId};
use internment::Intern;
use std::path::PathBuf;

use crate::path::ModPath;

/// One entry inside `use pkg.(a, b as c)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BundleExportImport {
    pub export: Intern<String>,
    pub alias: Option<Intern<String>>,
}

/// `use dep.(a, b as c)` — dependency `dep` must be in `flask.jsonc`; each member is a nested folder module under the dependency root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalBundleImport {
    pub root: Intern<String>,
    pub members: Vec<BundleExportImport>,
    /// Span covering the whole `root.(...)` construct for diagnostics / goto-def.
    pub span: SpanId,
}

/// `use` can include several different modules seperated by a `,`
///
/// ex.
/// ```gin
/// use http.web, crypto.hash
/// use './math.gin' as math
/// use utils.(math, http as h)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Import(pub Vec<ModuleImport>);

// TODO: for scripts we want to support git urls as if they were in flask.jsonc
// but in use statements so scripts can use remote depenecies
// `use 'https://github.com/gin/db_project.git' as db`

// TODO: explicit importing for reduces interface subscription
// `use core.http (...)`

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportSource {
    /// Top level name defined in `flask.jsonc` ex. `use http.*`
    Package(ModPath),
    /// Path to a module on disk ex. `use '../http' as http`
    Local(PathBuf, SpanId),
    /// Dependency bundle: `use core.(io, fs as store)`
    LocalBundle(LocalBundleImport),
}

/// An import is structured like the following:
///
/// `use {module_name}.path.to_sub_mod (import1, ImportTag)`
/// `use './local/folder' as alias`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleImport {
    pub source: ImportSource,
    pub alias: Option<Intern<String>>,
}

impl ModuleImport {
    /// Compute the default alias name from the import source.
    ///
    /// - `Package(path)` → last segment, or root if no segments
    /// - `Local(path)` → last component of the folder path
    pub fn effective_name(&self) -> String {
        match &self.source {
            ImportSource::Package(path) => path
                .segments
                .last()
                .map(|s| s.to_string())
                .unwrap_or_else(|| path.root.to_string()),
            ImportSource::Local(path, _) => path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            ImportSource::LocalBundle(b) => b.root.to_string(),
        }
    }
}

impl HasSpanId for ImportSource {
    fn span_id(&self) -> SpanId {
        match self {
            ImportSource::Package(mp) => mp.span_id(),
            ImportSource::Local(_, span_id) => *span_id,
            ImportSource::LocalBundle(b) => b.span_id(),
        }
    }
}

impl HasSpanId for ModuleImport {
    fn span_id(&self) -> SpanId {
        self.source.span_id()
    }
}

impl HasSpanId for LocalBundleImport {
    fn span_id(&self) -> SpanId {
        self.span
    }
}
