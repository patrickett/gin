mod compile_target;
mod config;
mod handle;
mod manifest;
mod minimum;
mod registry;
mod registry_client;
mod resolution;
mod resolve;
mod sandbox;
mod snapshot;
mod storage;
mod version;
mod workspace;

pub use compile_target::{CompileTarget, TargetError, TargetTriple};
pub use config::{Dependency, DependencyKind, FlaskConfig};
pub use handle::{ConfigError, FlaskConfigHandle, FlaskConfigReadGuard};
pub use manifest::{MANIFEST_NAME, ManifestError};
pub use minimum::MinimumRequirement;
pub use registry::{Registry, RegistryError, Release, ReleaseStatus, SecurityPolicy};
pub use registry_client::{HttpsTransport, RegistryClient, RegistrySettings, RegistryTransport};
pub use resolution::{LockedPackage, PackageResolution, ResolutionError, valid_package_name};
pub use resolve::{FlaskPathExt, NestedPackageResolveError, NestedPackageTarget};
pub use sandbox::ExecutionSandbox;
pub use snapshot::{
    MetadataError, SignedSnapshot, Snapshot, SnapshotSignature, TrustRoot, VerifiedSnapshot,
};
pub use storage::GinHome;
pub use target_lexicon::Architecture;
pub use version::{VersionBump, VersionError};
