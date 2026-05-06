use crate::DiagnosticLike;

#[derive(Debug, Clone, PartialEq, Eq, strum::AsRefStr)]
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
        }
    }

    fn help(&self) -> Option<String> {
        Some(match self {
            Self::Conflict { .. } => "choose a single qualifier/alias for this module".into(),
            Self::TargetNotFound { .. } => "ensure the import path points to an existing `.gin` file or folder module".into(),
            Self::LocalMustEndInGin { .. } => "use `use './file.gin'` for local file imports".into(),
            Self::LocalNotFound { .. } => "check the path relative to this file, and ensure it ends in `.gin`".into(),
            Self::FolderMissingConfig { .. } => "add a flask.jsonc to the folder module, or import a .gin file instead".into(),
            Self::MissingExport { .. } => "add a nested folder `segment/flask.jsonc` under the parent package".into(),
            Self::ExportTargetNotFound { .. } => "ensure the nested package path exists with a `flask.jsonc`".into(),
            Self::AmbiguousLocalRoot { .. } => "rename one of them, or use an explicit local file import (`use './path.gin'`)".into(),
            Self::FileHasSegments { .. } => "remove the trailing segment, or use a nested folder package".into(),
            Self::UnknownDependency { .. } => "add it to `dependencies` in flask.jsonc, or use a local file import".into(),
            Self::DependencyMissingConfig { .. } => "add a flask.jsonc to the dependency root directory".into(),
            Self::MissingConfig { .. } => "add a `flask.jsonc` in this folder module directory".into(),
            Self::ChainedExportNotFolder { .. } => "make the export's `path` point to a folder containing flask.jsonc, or stop the chain here".into(),
            Self::Cycle { chain } => format!("cycle: {chain}"),
            Self::LocalFolderRequiresAs { .. } => {
                "add `as Alias` so the folder module has a single namespace prefix".into()
            }
            Self::NestedPackageNotFound { .. } => {
                "create `segment/flask.jsonc` under the parent package, or fix the import path".into()
            }
            Self::PackageHasNoGinFiles { .. } => {
                "add at least one `.gin` file next to flask.jsonc".into()
            }
            Self::DuplicateTopLevel { .. } => {
                "rename or move one of the definitions so each public top-level name is unique in the package"
                    .into()
            }
            Self::NotExported { symbol, module } => format!(
                "`{symbol}` is not exported from `{module}`"
            ),
        })
    }
}
