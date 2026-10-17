mod conditional_import_arguments;
mod conditional_import_graph;
mod conditional_imports;
pub(crate) mod file_helpers;
pub(crate) mod folder_module;
mod front_end;
pub(crate) mod graph;
pub(crate) mod import_merge;
pub(crate) mod import_query;
pub(crate) mod import_suggest;
pub(crate) mod package_resolver;
pub(crate) mod public_symbols;
pub mod resolved_package_graph;
pub(crate) mod symbol_location;

mod module_graph;
mod module_inventory;
mod module_loader;

// Re-export batch pipeline
pub use front_end::{
    AssembledFile, AssembledPackage, FrontEndFile, PackageFrontEndInputs, PackageFrontEndOptions,
    PackageFrontEndOutput, PreparedPackageFrontEnd, check_package_front_end, default_check_session,
    prepare_package_front_end, run_package_front_end, run_package_front_end_with_session,
};
pub use module_inventory::child_folder_module_names;
pub use module_loader::ParsedModuleCache;
pub use package_resolver::{ImportDependencyGraph, resolve_imports};
pub use resolved_package_graph::{
    ResolvedDependencyEdge, ResolvedDependencyState, ResolvedPackageGraph, ResolvedPackageId,
    ResolvedPackageNode, UnresolvedDependency,
};
// Re-export per-request queries
pub use import_query::{
    ImportTarget, find_package_root, hover_for_import_target, resolve_dep_dir, resolve_dep_hover,
    resolve_import_at, resolve_local_symbol_hover,
};

pub use symbol_location::{
    CursorDefinition, DefLocation, ImportNavLocation, body_import_def_location,
    current_module_package_def_location, current_module_sibling_def_location, cursor_definition,
    def_location_for_import_target, dep_bundle_member_def_location, import_source_nav_location,
    is_import_identifier_at, local_bundle_def_location, local_definition_span,
};

// Re-export file helpers — use the trait for method syntax on `Path`.
pub use file_helpers::GinPackageExt;

pub use import_merge::MergedImportSuggestion;
pub use import_suggest::find_import_suggestions;

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
