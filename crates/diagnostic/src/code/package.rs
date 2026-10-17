use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum PackageCode {
    ConflictingResolvedInstance,
    DependencyCycle,
    GraphLimit,
    ImportEscapesRegistrySource,
    InvalidManifest,
    InvalidPathDependency,
    InvalidResolution,
    InvalidWorkspace,
    ManifestIdentityMismatch,
    RegistryVerificationFailed,
    UnresolvedGitRevision,
    UnresolvedRegistryInstance,
}

impl Prefix for PackageCode {
    fn prefix(&self) -> &'static str {
        "package"
    }
}

impl Code for PackageCode {
    fn name(&self) -> &'static str {
        match self {
            Self::ConflictingResolvedInstance => "conflicting-resolved-instance",
            Self::DependencyCycle => "dependency-cycle",
            Self::GraphLimit => "graph-limit",
            Self::ImportEscapesRegistrySource => "import-escapes-registry-source",
            Self::InvalidManifest => "invalid-manifest",
            Self::InvalidPathDependency => "invalid-path-dependency",
            Self::InvalidResolution => "invalid-resolution",
            Self::InvalidWorkspace => "invalid-workspace",
            Self::ManifestIdentityMismatch => "manifest-identity-mismatch",
            Self::RegistryVerificationFailed => "registry-verification-failed",
            Self::UnresolvedGitRevision => "unresolved-git-revision",
            Self::UnresolvedRegistryInstance => "unresolved-registry-instance",
        }
    }
}

impl PackageCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::ConflictingResolvedInstance,
            Self::DependencyCycle,
            Self::GraphLimit,
            Self::ImportEscapesRegistrySource,
            Self::InvalidManifest,
            Self::InvalidPathDependency,
            Self::InvalidResolution,
            Self::InvalidWorkspace,
            Self::ManifestIdentityMismatch,
            Self::RegistryVerificationFailed,
            Self::UnresolvedGitRevision,
            Self::UnresolvedRegistryInstance,
        ]
        .into_iter()
    }
}
