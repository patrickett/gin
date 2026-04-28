use strum::AsRefStr;

use crate::SpanId;
use crate::{Category, Symptom, SymptomCode, SymptomLike};

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

impl SymptomLike for ImportSymptom {
    fn into_symptom(self, span_id: SpanId) -> Symptom {
        let (message, help): (String, Option<String>) = match &self {
            Self::Conflict {
                path,
                qualifier_a,
                qualifier_b,
            } => (
                format!(
                    "import conflict: {} is pulled in as `{}` and `{}`",
                    path, qualifier_a, qualifier_b
                ),
                Some("choose a single qualifier/alias for this module".into()),
            ),
            Self::TargetNotFound { path } => (
                format!("import target not found: `{}`", path),
                Some(
                    "ensure the export `path` points to an existing `.gin` file (or a folder-module when importing a folder)"
                        .into(),
                ),
            ),
            Self::LocalMustEndInGin { path } => (
                format!("local import `{}` must end in `.gin`", path),
                Some("use `use './file.gin'` for local file imports".into()),
            ),
            Self::LocalNotFound { path } => (
                format!("local import not found: `{}`", path),
                Some(
                    "check the path relative to this file, and ensure it ends in `.gin`".into(),
                ),
            ),
            Self::FolderMissingConfig { folder } => (
                format!(
                    "`{}` is not a folder module (missing flask.jsonc)",
                    folder
                ),
                Some(
                    "add a flask.jsonc to the folder module, or import a .gin file instead".into(),
                ),
            ),
            Self::MissingExport { folder, export } => (
                format!("folder `{}` has no export `{}`", folder, export),
                Some("add this key to `exports` in flask.jsonc".into()),
            ),
            Self::ExportTargetNotFound {
                export,
                folder,
                path,
            } => (
                format!(
                    "export `{}` in `{}` points to missing path `{}`",
                    export, folder, path
                ),
                Some(
                    "fix the `path` in `exports` so it points to an existing file or folder-module"
                        .into(),
                ),
            ),
            Self::AmbiguousLocalRoot {
                name,
                file_path,
                folder_path,
            } => (
                format!(
                    "ambiguous `{}`: both `{}` and `{}/` exist",
                    name, file_path, folder_path
                ),
                Some(
                    "rename one of them, or use an explicit local file import (`use './path.gin'`)"
                        .into(),
                ),
            ),
            Self::FileHasSegments {
                file_path,
                segment,
            } => (
                format!(
                    "file module `{}` cannot have `{}` after it",
                    file_path, segment
                ),
                Some(
                    "remove the trailing segment, or import a folder-module with exports instead"
                        .into(),
                ),
            ),
            Self::UnknownDependency { name } => (
                format!(
                    "unknown dependency `{}` (not found in flask.jsonc dependencies)",
                    name
                ),
                Some(
                    "add it to `dependencies` in flask.jsonc, or use a local file import".into(),
                ),
            ),
            Self::DependencyMissingConfig { name, path } => (
                format!(
                    "dependency `{}` has no flask.jsonc at {}",
                    name, path
                ),
                Some("add a flask.jsonc to the dependency root directory".into()),
            ),
            Self::MissingConfig { dir } => (
                format!("missing flask.jsonc at `{}`", dir),
                Some("add a flask.jsonc with `exports` for this folder-module".into()),
            ),
            Self::ChainedExportNotFolder { path } => (
                format!(
                    "intermediate export resolved to non-folder-module `{}`",
                    path
                ),
                Some(
                    "make the export's `path` point to a folder containing flask.jsonc, or stop the chain here"
                        .into(),
                ),
            ),
            Self::Cycle { chain } => (
                "import cycle detected".into(),
                Some(format!("cycle: {chain}")),
            ),
        };

        Symptom {
            code: SymptomCode::Import(self),
            message,
            help,
            span_id,
            category: Category::Flaw,
        }
    }
}
