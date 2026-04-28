use strum::AsRefStr;

use crate::DiagnosticLike;

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum ImportSymptom {
    #[strum(to_string = "import-conflict")]
    Conflict {
        path: String,
        qualifier_a: String,
        qualifier_b: String,
    },
    #[strum(to_string = "import-target-not-found")]
    TargetNotFound {
        path: String,
    },
    #[strum(to_string = "import-local-must-end-in-gin")]
    LocalMustEndInGin {
        path: String,
    },
    #[strum(to_string = "import-local-not-found")]
    LocalNotFound {
        path: String,
    },
    #[strum(to_string = "import-folder-missing-config")]
    FolderMissingConfig {
        folder: String,
    },
    #[strum(to_string = "import-missing-export")]
    MissingExport {
        folder: String,
        export: String,
    },
    #[strum(to_string = "import-export-target-not-found")]
    ExportTargetNotFound {
        export: String,
        folder: String,
        path: String,
    },
    #[strum(to_string = "import-ambiguous-local-root")]
    AmbiguousLocalRoot {
        name: String,
        file_path: String,
        folder_path: String,
    },
    #[strum(to_string = "import-file-has-segments")]
    FileHasSegments {
        file_path: String,
        segment: String,
    },
    #[strum(to_string = "import-unknown-dependency")]
    UnknownDependency {
        name: String,
    },
    #[strum(to_string = "import-dependency-missing-config")]
    DependencyMissingConfig {
        name: String,
        path: String,
    },
    #[strum(to_string = "import-missing-config")]
    MissingConfig {
        dir: String,
    },
    #[strum(to_string = "import-chained-export-not-folder")]
    ChainedExportNotFolder {
        path: String,
    },
    #[strum(to_string = "import-cycle")]
    Cycle {
        chain: String,
    },
}

impl DiagnosticLike for ImportSymptom {
    fn message(&self) -> String {
        match self {
            Self::Conflict { path, qualifier_a, qualifier_b } => format!(
                "import conflict: {} is pulled in as `{}` and `{}`",
                path, qualifier_a, qualifier_b
            ),
            Self::TargetNotFound { path } => format!("import target not found: `{}`", path),
            Self::LocalMustEndInGin { path } => format!("local import `{}` must end in `.gin`", path),
            Self::LocalNotFound { path } => format!("local import not found: `{}`", path),
            Self::FolderMissingConfig { folder } => format!(
                "`{}` is not a folder module (missing flask.jsonc)", folder
            ),
            Self::MissingExport { folder, export } => format!("folder `{}` has no export `{}`", folder, export),
            Self::ExportTargetNotFound { export, folder, path } => format!(
                "export `{}` in `{}` points to missing path `{}`", export, folder, path
            ),
            Self::AmbiguousLocalRoot { name, file_path, folder_path } => format!(
                "ambiguous `{}`: both `{}` and `{}/` exist", name, file_path, folder_path
            ),
            Self::FileHasSegments { file_path, segment } => format!(
                "file module `{}` cannot have `{}` after it", file_path, segment
            ),
            Self::UnknownDependency { name } => format!(
                "unknown dependency `{}` (not found in flask.jsonc dependencies)", name
            ),
            Self::DependencyMissingConfig { name, path } => format!(
                "dependency `{}` has no flask.jsonc at {}", name, path
            ),
            Self::MissingConfig { dir } => format!("missing flask.jsonc at `{}`", dir),
            Self::ChainedExportNotFolder { path } => format!(
                "intermediate export resolved to non-folder-module `{}`", path
            ),
            Self::Cycle { chain: _ } => "import cycle detected".into(),
        }
    }

    fn help(&self) -> Option<String> {
        Some(match self {
            Self::Conflict { .. } => "choose a single qualifier/alias for this module".into(),
            Self::TargetNotFound { .. } => "ensure the export `path` points to an existing `.gin` file (or a folder-module when importing a folder)".into(),
            Self::LocalMustEndInGin { .. } => "use `use './file.gin'` for local file imports".into(),
            Self::LocalNotFound { .. } => "check the path relative to this file, and ensure it ends in `.gin`".into(),
            Self::FolderMissingConfig { .. } => "add a flask.jsonc to the folder module, or import a .gin file instead".into(),
            Self::MissingExport { .. } => "add this key to `exports` in flask.jsonc".into(),
            Self::ExportTargetNotFound { .. } => "fix the `path` in `exports` so it points to an existing file or folder-module".into(),
            Self::AmbiguousLocalRoot { .. } => "rename one of them, or use an explicit local file import (`use './path.gin'`)".into(),
            Self::FileHasSegments { .. } => "remove the trailing segment, or import a folder-module with exports instead".into(),
            Self::UnknownDependency { .. } => "add it to `dependencies` in flask.jsonc, or use a local file import".into(),
            Self::DependencyMissingConfig { .. } => "add a flask.jsonc to the dependency root directory".into(),
            Self::MissingConfig { .. } => "add a flask.jsonc with `exports` for this folder-module".into(),
            Self::ChainedExportNotFolder { .. } => "make the export's `path` point to a folder containing flask.jsonc, or stop the chain here".into(),
            Self::Cycle { chain } => format!("cycle: {chain}"),
        })
    }
}
