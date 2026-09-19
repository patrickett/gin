use interface::{Compatibility, PublicInterface, classify_interfaces};
use semver::Version;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionBump {
    Patch,
    Minor,
    Major,
}

#[derive(Debug, Error)]
pub enum VersionError {
    #[error("interfaces belong to different packages")]
    PackageMismatch,
    #[error("version checks require interfaces generated with the current schema")]
    SchemaMismatch,
    #[error("interface compatibility cannot be established")]
    UnknownCompatibility,
    #[error("version increment overflow")]
    Overflow,
    #[error("automatic bumps require a stable baseline version")]
    UnstableBaseline,
    #[error("version {actual} does not match the required version {expected}")]
    IncorrectVersion { expected: Version, actual: Version },
}

impl VersionBump {
    pub fn compare(old: &PublicInterface, new: &PublicInterface) -> Result<Self, VersionError> {
        if old.subject.package != new.subject.package {
            return Err(VersionError::PackageMismatch);
        }
        if old.schema_version != interface::SCHEMA_VERSION
            || new.schema_version != interface::SCHEMA_VERSION
        {
            return Err(VersionError::SchemaMismatch);
        }
        if old.target_profiles != new.target_profiles
            || old.resolution_fingerprint != new.resolution_fingerprint
        {
            return Ok(Self::Major);
        }
        if old.target_realizations != new.target_realizations
            || old.comptime_images != new.comptime_images
        {
            return Err(VersionError::UnknownCompatibility);
        }
        match classify_interfaces(old, new) {
            Compatibility::Unchanged
                if old.public_closure_fingerprint != new.public_closure_fingerprint
                    || old.semantic_surface_fingerprint != new.semantic_surface_fingerprint =>
            {
                Err(VersionError::UnknownCompatibility)
            }
            Compatibility::Unchanged => Ok(Self::Patch),
            Compatibility::Compatible => Ok(Self::Minor),
            Compatibility::Breaking => Ok(Self::Major),
            Compatibility::Unknown => Err(VersionError::UnknownCompatibility),
        }
    }

    pub fn next(self, old: &Version) -> Result<Version, VersionError> {
        if !old.pre.is_empty() || !old.build.is_empty() {
            return Err(VersionError::UnstableBaseline);
        }
        let increment = |n: u64| n.checked_add(1).ok_or(VersionError::Overflow);
        Ok(match self {
            Self::Patch => Version::new(old.major, old.minor, increment(old.patch)?),
            Self::Minor => Version::new(old.major, increment(old.minor)?, 0),
            Self::Major => Version::new(increment(old.major)?, 0, 0),
        })
    }

    pub fn verify(self, old: &Version, proposed: &Version) -> Result<(), VersionError> {
        let expected = self.next(old)?;
        if expected == *proposed {
            Ok(())
        } else {
            Err(VersionError::IncorrectVersion {
                expected,
                actual: proposed.clone(),
            })
        }
    }
}
