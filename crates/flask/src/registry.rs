use std::collections::BTreeMap;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{LockedPackage, PackageResolution, ResolutionError, VersionBump, VersionError};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub name: String,
    pub version: Version,
    pub archive_sha256: String,
    pub interface_sha256: String,
    pub published_at: u64,
    pub dependencies: BTreeMap<String, VersionReq>,
}

impl Release {
    pub fn key(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }

    pub fn verify_archive(&self, bytes: &[u8]) -> Result<(), RegistryError> {
        if format!("{:x}", Sha256::digest(bytes)) == self.archive_sha256 {
            Ok(())
        } else {
            Err(RegistryError::Integrity(self.key()))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ReleaseStatus {
    Quarantined,
    Eligible,
    Blocked { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityEvent {
    pub revision: u64,
    pub release: String,
    pub status: ReleaseStatus,
}

#[derive(Clone, Copy, Debug)]
pub struct SecurityPolicy {
    pub now: u64,
    pub metadata_expires_at: u64,
    pub cooldown_seconds: u64,
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("release `{0}` is immutable")]
    Immutable(String),
    #[error("invalid release record `{0}`")]
    InvalidRelease(String),
    #[error("unknown release `{0}`")]
    Missing(String),
    #[error("release `{0}` is blocked, quarantined, or still in cooldown")]
    Ineligible(String),
    #[error("release `{release}` is blocked: {reason}")]
    Blocked { release: String, reason: String },
    #[error("registry security metadata has expired")]
    Expired,
    #[error("release `{0}` failed integrity verification")]
    Integrity(String),
    #[error("no eligible dependency graph satisfies the requirements with one version per package")]
    Conflict,
    #[error("{0}")]
    Resolution(#[from] ResolutionError),
    #[error("{0}")]
    Version(#[from] VersionError),
    #[error("release interface is invalid: {0}")]
    Interface(String),
    #[error("dependency resolution exceeded its search limit")]
    SearchLimit,
}

#[derive(Default)]
/// Callers must authenticate imported metadata before populating this catalog.
/// Eligibility checks do not verify signatures or provide durable storage.
pub struct Registry {
    releases: BTreeMap<String, BTreeMap<Version, Release>>,
    status: BTreeMap<String, ReleaseStatus>,
    events: Vec<SecurityEvent>,
}

impl Registry {
    pub(crate) fn releases(&self) -> impl Iterator<Item = &Release> {
        self.releases
            .values()
            .flat_map(|versions| versions.values())
    }

    pub(crate) fn replace_events(&mut self, events: Vec<SecurityEvent>) {
        self.events = events;
    }
    pub fn security_events(&self) -> &[SecurityEvent] {
        &self.events
    }

    /// `compiled` must come from a trusted compiler run on `archive`.
    /// Digest checks alone cannot establish that source-to-interface relationship.
    pub fn validate_publication(
        &self,
        release: &Release,
        archive: &[u8],
        compiled: &interface::PublicInterface,
        previous: Option<&interface::PublicInterface>,
    ) -> Result<(), RegistryError> {
        release.verify_archive(archive)?;
        if self
            .releases
            .get(&release.name)
            .is_some_and(|versions| versions.contains_key(&release.version))
        {
            return Err(RegistryError::Immutable(release.key()));
        }
        if compiled.subject.package != release.name
            || compiled.subject.version != release.version.to_string()
            || format!("{:x}", Sha256::digest(compiled.canonical_bytes()))
                != release.interface_sha256
        {
            return Err(RegistryError::Integrity(release.key()));
        }
        match (self.releases.get(&release.name), previous) {
            (Some(versions), Some(old)) => {
                let version = Version::parse(&old.subject.version)
                    .map_err(|e| RegistryError::Interface(e.to_string()))?;
                let baseline = self.release(&release.name, &version)?;
                if versions.keys().next_back() != Some(&version)
                    || format!("{:x}", Sha256::digest(old.canonical_bytes()))
                        != baseline.interface_sha256
                {
                    return Err(RegistryError::Integrity(baseline.key()));
                }
                VersionBump::compare(old, compiled)?.verify(&version, &release.version)?;
            }
            (None, None) if release.version == Version::new(1, 0, 0) => {}
            _ => {
                return Err(RegistryError::Interface(
                    "provide the latest published interface, or publish a new package at 1.0.0"
                        .into(),
                ));
            }
        }
        Ok(())
    }

    pub fn insert(&mut self, release: Release) -> Result<(), RegistryError> {
        let valid_digest = |s: &str| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if !crate::valid_package_name(&release.name)
            || !valid_digest(&release.archive_sha256)
            || !valid_digest(&release.interface_sha256)
            || !release.version.build.is_empty()
            || release
                .version
                .pre
                .as_str()
                .bytes()
                .any(|b| b.is_ascii_uppercase())
        {
            return Err(RegistryError::InvalidRelease(release.key()));
        }
        let versions = self.releases.entry(release.name.clone()).or_default();
        if versions.contains_key(&release.version) {
            return Err(RegistryError::Immutable(release.key()));
        }
        self.status
            .insert(release.key(), ReleaseStatus::Quarantined);
        self.events.push(SecurityEvent {
            revision: self.events.len() as u64 + 1,
            release: release.key(),
            status: ReleaseStatus::Quarantined,
        });
        versions.insert(release.version.clone(), release);
        Ok(())
    }

    pub fn set_status(
        &mut self,
        name: &str,
        version: &Version,
        status: ReleaseStatus,
    ) -> Result<(), RegistryError> {
        let key = self.release(name, version)?.key();
        self.events.push(SecurityEvent {
            revision: self.events.len() as u64 + 1,
            release: key.clone(),
            status: status.clone(),
        });
        self.status.insert(key, status);
        Ok(())
    }

    pub fn release(&self, name: &str, version: &Version) -> Result<&Release, RegistryError> {
        self.releases
            .get(name)
            .and_then(|versions| versions.get(version))
            .ok_or_else(|| RegistryError::Missing(format!("{name}@{version}")))
    }

    pub fn check(&self, release: &Release, policy: SecurityPolicy) -> Result<(), RegistryError> {
        if policy.now >= policy.metadata_expires_at {
            return Err(RegistryError::Expired);
        }
        if self.release(&release.name, &release.version)? != release {
            return Err(RegistryError::Integrity(release.key()));
        }
        if let Some(ReleaseStatus::Blocked { reason }) = self.status.get(&release.key()) {
            return Err(RegistryError::Blocked {
                release: release.key(),
                reason: reason.clone(),
            });
        }
        if self.status.get(&release.key()) != Some(&ReleaseStatus::Eligible)
            || policy
                .now
                .checked_sub(release.published_at)
                .is_none_or(|age| age < policy.cooldown_seconds)
        {
            return Err(RegistryError::Ineligible(release.key()));
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        requirements: &BTreeMap<String, VersionReq>,
        policy: SecurityPolicy,
    ) -> Result<PackageResolution, RegistryError> {
        if policy.now >= policy.metadata_expires_at {
            return Err(RegistryError::Expired);
        }
        let pending = requirements
            .iter()
            .map(|(name, req)| (name.clone(), req.clone()))
            .collect();
        let mut remaining = 10_000;
        let selected = self
            .solve(pending, BTreeMap::new(), policy, &mut remaining)
            .ok_or(if remaining == 0 {
                RegistryError::SearchLimit
            } else {
                RegistryError::Conflict
            })?;
        let resolution = PackageResolution {
            direct: requirements
                .keys()
                .map(|name| (name.clone(), selected[name].key()))
                .collect(),
            packages: selected
                .values()
                .map(|release| {
                    (
                        release.key(),
                        LockedPackage {
                            name: release.name.clone(),
                            version: release.version.clone(),
                            sha256: release.archive_sha256.clone(),
                            dependencies: release
                                .dependencies
                                .keys()
                                .map(|name| (name.clone(), selected[name].key()))
                                .collect(),
                        },
                    )
                })
                .collect(),
        };
        self.validate_resolution(requirements, &resolution, policy)?;
        Ok(resolution)
    }

    fn solve<'a>(
        &'a self,
        mut pending: Vec<(String, VersionReq)>,
        selected: BTreeMap<String, &'a Release>,
        policy: SecurityPolicy,
        remaining: &mut usize,
    ) -> Option<BTreeMap<String, &'a Release>> {
        if *remaining == 0 || selected.len() > 256 || pending.len() > 4096 {
            *remaining = 0;
            return None;
        }
        *remaining -= 1;
        let Some((name, req)) = pending.pop() else {
            return acyclic(&selected).then_some(selected);
        };
        if let Some(release) = selected.get(&name) {
            return req
                .matches(&release.version)
                .then(|| self.solve(pending, selected, policy, remaining))
                .flatten();
        }
        for release in self.releases.get(&name)?.values().rev() {
            if !req.matches(&release.version) || self.check(release, policy).is_err() {
                continue;
            }
            let mut selected = selected.clone();
            selected.insert(name.clone(), release);
            let mut next = pending.clone();
            next.extend(
                release
                    .dependencies
                    .iter()
                    .map(|(name, req)| (name.clone(), req.clone())),
            );
            if let Some(solution) = self.solve(next, selected, policy, remaining) {
                return Some(solution);
            }
        }
        None
    }

    pub fn validate_resolution(
        &self,
        requirements: &BTreeMap<String, VersionReq>,
        resolution: &PackageResolution,
        policy: SecurityPolicy,
    ) -> Result<(), RegistryError> {
        if policy.now >= policy.metadata_expires_at {
            return Err(RegistryError::Expired);
        }
        resolution.validate(requirements)?;
        for pkg in resolution.packages.values() {
            let release = self.release(&pkg.name, &pkg.version)?;
            self.check(release, policy)?;
            if release.archive_sha256 != pkg.sha256 {
                return Err(RegistryError::Integrity(release.key()));
            }
            if release.dependencies.len() != pkg.dependencies.len() {
                return Err(RegistryError::Resolution(ResolutionError(format!(
                    "dependency graph differs for `{}`",
                    release.key()
                ))));
            }
            for (name, req) in &release.dependencies {
                let child = pkg
                    .dependencies
                    .get(name)
                    .and_then(|key| resolution.packages.get(key))
                    .ok_or_else(|| {
                        RegistryError::Resolution(ResolutionError(format!(
                            "missing dependency `{name}`"
                        )))
                    })?;
                if child.name != *name || !req.matches(&child.version) {
                    return Err(RegistryError::Resolution(ResolutionError(format!(
                        "dependency `{name}` does not satisfy `{req}`"
                    ))));
                }
            }
        }
        Ok(())
    }
}

fn acyclic(selected: &BTreeMap<String, &Release>) -> bool {
    let mut remaining: std::collections::BTreeSet<_> = selected.keys().collect();
    loop {
        if remaining.is_empty() {
            return true;
        }
        let leaf = remaining
            .iter()
            .find(|name| {
                selected
                    .get(**name)
                    .unwrap()
                    .dependencies
                    .keys()
                    .all(|dep| !remaining.contains(dep))
            })
            .copied();
        match leaf {
            Some(name) => {
                remaining.remove(name);
            }
            None => return false,
        }
    }
}
