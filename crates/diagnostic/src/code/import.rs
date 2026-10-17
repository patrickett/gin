use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum ImportCode {
    ConditionNotCompileTime,
    Conflict,
    Cycle,
    DepLocalNameCollision,
    DependencyMissingConfig,
    DiscoveryLimit,
    EscapesPackageRoot,
    ImportTargetMustBeFolder,
    LocalFileNotFound,
    LocalFolderRequiresAs,
    LocalNotFound,
    ModuleNotFound,
    NestedPackageNotFound,
    NotExported,
    NotResolved,
    PackageNoGinFiles,
    SpecializationArgumentDuplicate,
    SpecializationArgumentName,
    SpecializationArity,
    SpecializationLimit,
    StagingLimit,
    TargetNotFound,
    UnknownDependency,
    UnusedImport,
}

impl Prefix for ImportCode {
    fn prefix(&self) -> &'static str {
        "use"
    }
}

impl Code for ImportCode {
    fn name(&self) -> &'static str {
        match self {
            Self::ConditionNotCompileTime => "condition-not-compile-time",
            Self::Conflict => "conflict",
            Self::Cycle => "cycle",
            Self::DepLocalNameCollision => "dep-local-name-collision",
            Self::DependencyMissingConfig => "dependency-missing-config",
            Self::DiscoveryLimit => "discovery-limit",
            Self::EscapesPackageRoot => "escapes-package-root",
            Self::ImportTargetMustBeFolder => "import-target-must-be-folder",
            Self::LocalFileNotFound => "local-file-not-found",
            Self::LocalFolderRequiresAs => "local-folder-requires-as",
            Self::LocalNotFound => "local-not-found",
            Self::ModuleNotFound => "module-not-found",
            Self::NestedPackageNotFound => "nested-package-not-found",
            Self::NotExported => "not-exported",
            Self::NotResolved => "not-resolved",
            Self::PackageNoGinFiles => "package-no-gin-files",
            Self::SpecializationArgumentDuplicate => "specialization-argument-duplicate",
            Self::SpecializationArgumentName => "specialization-argument-name",
            Self::SpecializationArity => "specialization-arity",
            Self::SpecializationLimit => "specialization-limit",
            Self::StagingLimit => "staging-limit",
            Self::TargetNotFound => "target-not-found",
            Self::UnknownDependency => "unknown-dependency",
            Self::UnusedImport => "unused-import",
        }
    }
}

impl ImportCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::ConditionNotCompileTime,
            Self::Conflict,
            Self::Cycle,
            Self::DepLocalNameCollision,
            Self::DependencyMissingConfig,
            Self::DiscoveryLimit,
            Self::EscapesPackageRoot,
            Self::ImportTargetMustBeFolder,
            Self::LocalFileNotFound,
            Self::LocalFolderRequiresAs,
            Self::LocalNotFound,
            Self::ModuleNotFound,
            Self::NestedPackageNotFound,
            Self::NotExported,
            Self::NotResolved,
            Self::PackageNoGinFiles,
            Self::SpecializationArgumentDuplicate,
            Self::SpecializationArgumentName,
            Self::SpecializationArity,
            Self::SpecializationLimit,
            Self::StagingLimit,
            Self::TargetNotFound,
            Self::UnknownDependency,
            Self::UnusedImport,
        ]
        .into_iter()
    }
}
