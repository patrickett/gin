use crate::DiagnosticLike;

#[derive(Debug, Clone, PartialEq, Eq, Hash, strum::AsRefStr)]
#[non_exhaustive]
pub enum UseSymptom {
    #[strum(serialize = "use-conflict")]
    Conflict {
        path: String,
        qualifier_a: String,
        qualifier_b: String,
    },
    #[strum(serialize = "use-target-not-found")]
    TargetNotFound { path: String },
    #[strum(serialize = "use-local-must-end-in-gin")]
    LocalMustEndInGin { path: String },
    #[strum(serialize = "use-local-not-found")]
    LocalNotFound { path: String },
    #[strum(serialize = "use-folder-missing-config")]
    FolderMissingConfig { folder: String },
    #[strum(serialize = "use-missing-export")]
    MissingExport { folder: String, export: String },
    #[strum(serialize = "use-export-target-not-found")]
    ExportTargetNotFound {
        export: String,
        folder: String,
        path: String,
    },
    #[strum(serialize = "use-ambiguous-local-root")]
    AmbiguousLocalRoot {
        name: String,
        file_path: String,
        folder_path: String,
    },
    #[strum(serialize = "use-file-has-segments")]
    FileHasSegments { file_path: String, segment: String },
    #[strum(serialize = "use-unknown-dependency")]
    UnknownDependency { name: String },
    #[strum(serialize = "use-dependency-missing-config")]
    DependencyMissingConfig { name: String, path: String },
    #[strum(serialize = "use-missing-config")]
    MissingConfig { dir: String },
    #[strum(serialize = "use-chained-export-not-folder")]
    ChainedExportNotFolder { path: String },
    #[strum(serialize = "use-cycle")]
    Cycle { chain: String },
    #[strum(serialize = "use-local-folder-requires-as")]
    LocalFolderRequiresAs { path: String },
    #[strum(serialize = "use-nested-package-not-found")]
    NestedPackageNotFound { parent: String, segment: String },
    #[strum(serialize = "use-package-no-gin-files")]
    PackageHasNoGinFiles { dir: String },
    #[strum(serialize = "use-duplicate-top-level")]
    DuplicateTopLevel { symbol: String },
    /// A bundle member (e.g. `true` in `use core.(true)`) is not a sub-package
    /// and is not a public definition in the dependency's source files.
    #[strum(serialize = "use-not-exported")]
    NotExported {
        /// The name of the symbol that was requested.
        symbol: String,
        /// The module/dependency that was queried.
        module: String,
    },
    /// A `.gin` file that is not inside any recognised package (no
    /// `flask.jsonc` found in its parent directory chain).
    #[strum(serialize = "use-file-outside-package")]
    FileOutsidePackage {
        /// The path to the orphaned file.
        path: String,
    },
    #[strum(serialize = "use-import-target-must-be-folder")]
    ImportTargetMustBeFolder { path: String },
    #[strum(serialize = "use-escapes-package-root")]
    EscapesPackageRoot { path: String },
    #[strum(serialize = "use-dep-local-name-collision")]
    DepLocalNameCollision {
        name: String,
        dependency: String,
        local_path: String,
    },
    #[strum(serialize = "use-unused-import")]
    UnusedImport { name: String },
    /// Single-symbol bundle import such as `use core.(Int)` — prefer `use core.Int`.
    #[strum(serialize = "use-prefer-member-import")]
    PreferMemberImport { path_prefix: String, symbol: String },
}

impl DiagnosticLike for UseSymptom {
    fn message(&self) -> String {
        match self {
            Self::Conflict {
                path,
                qualifier_a,
                qualifier_b,
            } => format!(
                "import conflict: {} is pulled in as `{}` and `{}`",
                path, qualifier_a, qualifier_b
            ),
            Self::TargetNotFound { path } => format!("import target not found: `{}`", path),
            Self::LocalMustEndInGin { path } => {
                format!("local import `{}` must end in `.gin`", path)
            }
            Self::LocalNotFound { path } => format!("local import not found: `{}`", path),
            Self::FolderMissingConfig { folder } => {
                format!("`{}` is not a folder module (missing flask.jsonc)", folder)
            }
            Self::MissingExport { folder, export } => {
                format!("folder `{}` has no export `{}`", folder, export)
            }
            Self::ExportTargetNotFound {
                export,
                folder,
                path,
            } => format!(
                "export `{}` in `{}` points to missing path `{}`",
                export, folder, path
            ),
            Self::AmbiguousLocalRoot {
                name,
                file_path,
                folder_path,
            } => format!(
                "ambiguous `{}`: both `{}` and `{}/` exist",
                name, file_path, folder_path
            ),
            Self::FileHasSegments { file_path, segment } => format!(
                "file module `{}` cannot have `{}` after it",
                file_path, segment
            ),
            Self::UnknownDependency { name } => format!(
                "unknown dependency `{}` (not found in flask.jsonc dependencies)",
                name
            ),
            Self::DependencyMissingConfig { name, path } => {
                format!("dependency `{}` has no flask.jsonc at {}", name, path)
            }
            Self::MissingConfig { dir } => format!("missing flask.jsonc at `{}`", dir),
            Self::ChainedExportNotFolder { path } => format!(
                "intermediate export resolved to non-folder-module `{}`",
                path
            ),
            Self::Cycle { chain: _ } => "import cycle detected".into(),
            Self::LocalFolderRequiresAs { path } => format!(
                "folder module `{}` must be imported with `as` (e.g. `use '{}' as name`)",
                path, path
            ),
            Self::NestedPackageNotFound { parent, segment } => format!(
                "no nested package `{}/{}` (expected a folder module with flask.jsonc)",
                parent, segment
            ),
            Self::PackageHasNoGinFiles { dir } => {
                format!("folder module `{}` contains no `.gin` source files", dir)
            }
            Self::DuplicateTopLevel { symbol } => format!(
                "duplicate top-level definition `{}` when merging module files",
                symbol
            ),
            Self::NotExported { symbol, module } => {
                format!("`{}` is not exported from `{}`", symbol, module)
            }
            Self::FileOutsidePackage { path } => {
                format!(
                    "`{}` is not part of any package (no flask.jsonc found)",
                    path
                )
            }
            Self::ImportTargetMustBeFolder { path } => {
                format!("import target must be a folder module, not `{}`", path)
            }
            Self::EscapesPackageRoot { path } => {
                format!("import path `{}` escapes the package root", path)
            }
            Self::DepLocalNameCollision {
                name,
                dependency,
                local_path,
            } => format!(
                "name `{name}` conflicts with dependency `{dependency}` and local folder `{local_path}`; alias one side"
            ),
            Self::UnusedImport { name } => format!("unused import `{name}`"),
            Self::PreferMemberImport {
                path_prefix,
                symbol,
            } => format!("prefer `use {path_prefix}.{symbol}` over a single-item `.(…)` bundle"),
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::FileOutsidePackage { .. } => Some("add a `flask.jsonc` to this directory or one of its parents to make it a Gin package".into()),
            Self::ImportTargetMustBeFolder { .. } => {
                Some("import a directory (folder module), not a single `.gin` file".into())
            }
            Self::EscapesPackageRoot { .. } => {
                Some("use a path that stays within the package containing flask.jsonc".into())
            }
            Self::DepLocalNameCollision { name, .. } => Some(format!(
                "use `use {name} as {name}_dep` for the dependency, or `use '{name}' as {name}` for the local folder"
            )),
            Self::UnusedImport { .. } => Some("remove the import or reference the name in this file".into()),
            Self::PreferMemberImport { path_prefix, symbol } => Some(format!(
                "use `use {path_prefix}.{symbol}` for a single import; reserve `.(…)` for multiple symbols"
            )),
            Self::Conflict { .. } => Some("choose a single qualifier/alias for this module".into()),
            Self::TargetNotFound { .. } => Some("ensure the import path points to an existing `.gin` file or folder module".into()),
            Self::LocalMustEndInGin { .. } => Some("use `use './file.gin'` for local file imports".into()),
            Self::LocalNotFound { .. } => Some("check the path relative to this file, and ensure it ends in `.gin`".into()),
            Self::FolderMissingConfig { .. } => Some("add a flask.jsonc to the folder module, or import a .gin file instead".into()),
            Self::MissingExport { .. } => Some("add a nested folder `segment/flask.jsonc` under the parent package".into()),
            Self::ExportTargetNotFound { .. } => Some("ensure the nested package path exists with a `flask.jsonc`".into()),
            Self::AmbiguousLocalRoot { .. } => Some("rename one of them, or use an explicit local file import (`use './path.gin'`)".into()),
            Self::FileHasSegments { .. } => Some("remove the trailing segment, or use a nested folder package".into()),
            Self::UnknownDependency { .. } => Some("add it to `dependencies` in flask.jsonc, or use a local file import".into()),
            Self::DependencyMissingConfig { .. } => Some("add a flask.jsonc to the dependency root directory".into()),
            Self::MissingConfig { .. } => Some("add a `flask.jsonc` in this folder module directory".into()),
            Self::ChainedExportNotFolder { .. } => Some("make the export's `path` point to a folder containing flask.jsonc, or stop the chain here".into()),
            Self::Cycle { chain } => Some(format!("cycle: {chain}")),
            Self::LocalFolderRequiresAs { .. } => Some("add `as Alias` so the folder module has a single namespace prefix".into()),
            Self::NestedPackageNotFound { .. } => Some("create `segment/flask.jsonc` under the parent package, or fix the import path".into()),
            Self::PackageHasNoGinFiles { .. } => Some("add at least one `.gin` file next to flask.jsonc".into()),
            Self::DuplicateTopLevel { .. } => Some("rename or move one of the definitions so each public top-level name is unique in the package".into()),
            Self::NotExported { symbol, module } => Some(format!("`{symbol}` is not exported from `{module}`")),
        }
    }

    fn category(&self) -> crate::Category {
        match self {
            Self::FileOutsidePackage { .. } => crate::Category::Info,
            Self::UnusedImport { .. } => crate::Category::Help,
            _ => crate::Category::Flaw,
        }
    }
}
