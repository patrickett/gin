pub(crate) mod file_helpers;
pub(crate) mod folder_module;
pub(crate) mod graph;
pub(crate) mod import_merge;
pub(crate) mod import_query;
pub(crate) mod import_suggest;
pub(crate) mod package_resolver;
pub(crate) mod public_symbols;
pub(crate) mod symbol_location;

mod module_graph;
mod module_inventory;
mod module_loader;

// Re-export batch pipeline
pub use graph::{ResolveGraph, ResolveNode};
pub use module_loader::ParsedModuleCache;
pub use package_resolver::{
    ImportDependencyGraph, ResolveImportsWithGraph, resolve_import_symptoms, resolve_imports,
    resolve_imports_with_graph,
};
pub use public_symbols::find_public_def;
// Re-export per-request queries
pub use import_query::{
    ImportTarget, def_span_for_import_target, find_package_root, hover_for_import_target,
    part_index_in_dotted_path, resolve_current_module_def_span, resolve_current_module_hover,
    resolve_dep_dir, resolve_dep_hover, resolve_import_at, resolve_local_symbol_def_span,
    resolve_local_symbol_hover, resolve_symbol_def_span, resolve_symbol_hover,
};

pub use symbol_location::{
    CursorDefinition, DefLocation, ImportNavLocation, body_import_def_location,
    current_module_package_def_location, current_module_sibling_def_location, cursor_definition,
    def_location_for_import_target, dep_bundle_member_def_location, dep_symbol_def_location,
    import_source_nav_location, is_import_identifier_at, local_bundle_def_location,
    package_import_part_location, public_symbol_def_location,
};

// Re-export file helpers — use the trait for method syntax on `Path`.
pub use file_helpers::GinPackageExt;

pub use import_merge::{
    CollectedImport, ImportRoot, MergedImportSuggestion, RawSuggestion, collect_member_imports,
};
pub use import_suggest::{ImportSuggestion, find_import_suggestions};

pub use folder_module::{
    NormalizePathError, normalize_local_import_path, resolve_logical_module_dir,
    split_dep_path_segments,
};

use parser::query::{ParseOutput, SourceParseExt};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct ParsedFile {
    pub path: PathBuf,
    pub source: String,
    pub output: ParseOutput,
}

impl ParsedFile {
    pub fn filename(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    /// Parse a `.gin` file at the given path into a `ParsedFile`.
    pub fn read(path: &Path) -> Option<Self> {
        let source = std::fs::read_to_string(path).ok()?;
        let output = source.parse_source_full();
        Some(Self {
            path: path.to_path_buf(),
            source,
            output,
        })
    }
}
