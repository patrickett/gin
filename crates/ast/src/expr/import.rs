use crate::span::{HasSpanId, SpanId};
use internment::Intern;
use std::path::PathBuf;

use crate::path::ModPath;
use crate::span::Spanned;

/// One entry inside `use pkg.(a, b as c)` or `use 'path'.(a, b as c)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BundleExportImport {
    pub export: Intern<String>,
    pub alias: Option<Intern<String>>,
    /// The span of the export name (with alias if present) within the
    /// `.(...)` group. Used for per-member diagnostics and goto-def.
    pub span: SpanId,
}

/// `use dep.(a, b as c)` — dependency `dep` must be in `flask.jsonc`.
/// Also supports `use 'path'.(a, b as c)` — when `local_path` is set,
/// the import resolves against the local filesystem path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalBundleImport {
    /// Dependency name from flask.jsonc (e.g. "core" in `use core.(…)`).
    pub root: Intern<String>,
    pub members: Vec<BundleExportImport>,
    /// Span covering the whole `root.(...)` construct for diagnostics / goto-def.
    pub span: SpanId,
    /// When set, this is a `use 'path'.(items)` import. The resolver looks up
    /// `path` on the local filesystem instead of in flask.jsonc dependencies.
    pub local_path: Option<PathBuf>,
}

/// `use` can include several different modules seperated by a `,`
///
/// ex.
/// ```gin
/// use http.web, crypto.hash
/// use './math.gin' as math
/// use utils.(math, http as h)
/// use './path'.(item1, item2)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Import(pub Vec<ModuleImport>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportSource {
    /// Top level name defined in `flask.jsonc` ex. `use http.*`
    Package(Spanned<ModPath>),
    /// Path to a module on disk ex. `use '../http' as http`
    Local(PathBuf, SpanId),
    /// Dependency or local-path bundle: `use core.(io, fs as store)`
    /// or `use './path'.(item1, item2)`.
    LocalBundle(LocalBundleImport),
    /// `use Int` or `use to_string` — imports a symbol from the current
    /// module (same package, no path prefix).
    CurrentModule { member: BundleExportImport },
}

/// An import is structured like the following:
///
/// `use {module_name}.path.to_sub_mod (import1, ImportTag)`
/// `use './local/folder' as alias`
/// `use './local/folder'.(item1, item2)`
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
    /// - `LocalBundle(b)` → the root name
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
            ImportSource::LocalBundle(b) if b.local_path.is_some() => b
                .local_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            ImportSource::LocalBundle(b) => b.root.to_string(),
            ImportSource::CurrentModule { member } => member.export.to_string(),
        }
    }
}

impl HasSpanId for ImportSource {
    fn span_id(&self) -> SpanId {
        match self {
            ImportSource::Package(mp) => mp.span_id,
            ImportSource::Local(_, span_id) => *span_id,
            ImportSource::LocalBundle(b) => b.span_id(),
            ImportSource::CurrentModule { member } => member.span,
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
