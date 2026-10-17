use std::collections::{BTreeMap, BTreeSet, VecDeque};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    LockedPackage, PackageResolution, Registry, RegistryError, ResolutionError, SecurityPolicy,
};

const RELEASES_MAX: usize = 4096;
const REQUIREMENTS_MAX: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinimumRequirement {
    pub name: String,
    pub version: Version,
}

impl MinimumRequirement {
    pub fn from_legacy(alias: &str, text: &str) -> Result<Self, ResolutionError> {
        let requirement =
            semver::VersionReq::parse(text).map_err(|e| ResolutionError(e.to_string()))?;
        legacy_minimum(alias, &requirement)
    }
    pub fn parse(alias: &str, text: &str) -> Result<Self, ResolutionError> {
        let (name, version) = text.split_once('@').unwrap_or((alias, text));
        let requirement = Self {
            name: name.to_owned(),
            version: Version::parse(version).map_err(|e| ResolutionError(e.to_string()))?,
        };
        requirement.validate()?;
        Ok(requirement)
    }

    pub fn validate(&self) -> Result<(), ResolutionError> {
        if !crate::valid_package_name(&self.name)
            || self.version.major == 0
            || !self.version.pre.is_empty()
            || !self.version.build.is_empty()
        {
            return Err(ResolutionError(format!(
                "minimum requirement must name a stable release at 1.0.0 or later: {}@{}",
                self.name, self.version
            )));
        }
        Ok(())
    }

    pub fn identity(&self) -> (String, u64) {
        (self.name.clone(), self.version.major)
    }

    pub fn manifest_value(&self, alias: &str) -> String {
        if alias == self.name {
            self.version.to_string()
        } else {
            format!("{}@{}", self.name, self.version)
        }
    }
}

impl PackageResolution {
    pub fn minimum_key(&self, requirement: &MinimumRequirement) -> Option<&String> {
        let prefix = format!("{}@{}.", requirement.name, requirement.version.major);
        self.packages
            .range(prefix.clone()..)
            .next()
            .filter(|(key, package)| {
                key.starts_with(&prefix) && package.version >= requirement.version
            })
            .map(|(key, _)| key)
    }
}

impl Registry {
    pub fn update_minimum(
        &self,
        requirements: &BTreeMap<String, MinimumRequirement>,
        target: Option<&MinimumRequirement>,
        policy: SecurityPolicy,
    ) -> Result<PackageResolution, RegistryError> {
        policy.ensure_fresh()?;
        if let Some(target) = target {
            target.validate()?;
        }
        let selected = self.minimum_build_list(requirements)?;
        let mut floors = requirements.clone();
        for ((name, major), release) in selected {
            let alias = requirements
                .iter()
                .find(|(_, r)| r.name == name && r.version.major == major)
                .map(|(alias, _)| alias.clone())
                .unwrap_or_else(|| format!("{name}_v{major}"));
            if floors
                .get(&alias)
                .is_some_and(|r| r.name != name || r.version.major != major)
            {
                return Err(ResolutionError(format!("dependency alias collision: {alias}")).into());
            }
            let version = match target {
                Some(target) if target.name == name && target.version.major == major => {
                    if target.version < release.version {
                        return Err(ResolutionError(
                            "update cannot lower a selected version".into(),
                        )
                        .into());
                    }
                    target.version.clone()
                }
                Some(_) => release.version.clone(),
                None => self.latest_eligible(&name, major, policy)?.version.clone(),
            };
            floors.insert(alias, MinimumRequirement { name, version });
        }
        if let Some(target) = target
            && !floors.values().any(|r| r.identity() == target.identity())
        {
            return Err(
                ResolutionError("update target is not a dependency; use begin add".into()).into(),
            );
        }
        self.resolve_minimum(&floors, policy)
    }
    pub fn resolve_minimum(
        &self,
        requirements: &BTreeMap<String, MinimumRequirement>,
        policy: SecurityPolicy,
    ) -> Result<PackageResolution, RegistryError> {
        policy.ensure_fresh()?;
        let selected = self.minimum_build_list(requirements)?;
        let mut packages = BTreeMap::new();
        for release in selected.values() {
            self.check(release, policy)?;
            let dependencies = self.minimum_release_requirements(release)?;
            packages.insert(
                release.key(),
                LockedPackage {
                    name: release.name.clone(),
                    version: release.version.clone(),
                    sha256: release.archive_sha256.clone(),
                    dependencies: minimum_edges(&dependencies, &selected),
                },
            );
        }
        Ok(PackageResolution {
            direct: minimum_edges(requirements, &selected),
            packages,
        })
    }

    fn minimum_build_list(
        &self,
        requirements: &BTreeMap<String, MinimumRequirement>,
    ) -> Result<BTreeMap<(String, u64), &crate::Release>, RegistryError> {
        if requirements.len() > REQUIREMENTS_MAX {
            return Err(RegistryError::SearchLimit);
        }
        if requirements
            .keys()
            .any(|alias| !crate::valid_package_name(alias))
        {
            return Err(ResolutionError("invalid dependency alias".into()).into());
        }
        let mut pending: VecDeque<_> = requirements.values().cloned().collect();
        let mut visited = BTreeSet::new();
        let mut selected: BTreeMap<(String, u64), &crate::Release> = BTreeMap::new();
        let mut edges = pending.len();
        if edges > REQUIREMENTS_MAX {
            return Err(RegistryError::SearchLimit);
        }
        while let Some(requirement) = pending.pop_front() {
            requirement.validate()?;
            if !visited.insert((requirement.name.clone(), requirement.version.clone())) {
                continue;
            }
            if visited.len() > RELEASES_MAX {
                return Err(RegistryError::SearchLimit);
            }
            let release = self.release(&requirement.name, &requirement.version)?;
            let identity = requirement.identity();
            if selected
                .get(&identity)
                .is_none_or(|old| old.version < release.version)
            {
                selected.insert(identity, release);
            }
            let dependencies = self.minimum_release_requirements(release)?;
            edges += dependencies.len();
            if edges > REQUIREMENTS_MAX {
                return Err(RegistryError::SearchLimit);
            }
            pending.extend(dependencies.into_values());
        }
        Ok(selected)
    }

    pub(crate) fn minimum_release_requirements(
        &self,
        release: &crate::Release,
    ) -> Result<BTreeMap<String, MinimumRequirement>, RegistryError> {
        if let Some(requirements) = &release.minimum_dependencies {
            return Ok(requirements.clone());
        }
        release
            .dependencies
            .iter()
            .map(|(name, requirement)| {
                let parsed = legacy_minimum(name, requirement).map_err(|e| {
                    RegistryError::Resolution(ResolutionError(format!("{}: {e}", release.key())))
                })?;
                Ok((name.clone(), parsed))
            })
            .collect()
    }

    pub(crate) fn latest_eligible(
        &self,
        name: &str,
        major: u64,
        policy: SecurityPolicy,
    ) -> Result<&crate::Release, RegistryError> {
        self.releases_for(name)
            .rev()
            .find(|release| {
                release.version.major == major
                    && release.version.pre.is_empty()
                    && release.version.build.is_empty()
                    && self.check(release, policy).is_ok()
            })
            .ok_or_else(|| RegistryError::Missing(format!("eligible {name} major {major}")))
    }
}

pub(crate) fn legacy_minimum(
    name: &str,
    requirement: &semver::VersionReq,
) -> Result<MinimumRequirement, ResolutionError> {
    let comparator = requirement.comparators.first();
    let Some(comparator) = comparator.filter(|c| {
        requirement.comparators.len() == 1
            && c.op == semver::Op::Caret
            && c.major > 0
            && c.pre.is_empty()
    }) else {
        return Err(ResolutionError(
            "legacy constraint needs republication with minimum requirements".into(),
        ));
    };
    Ok(MinimumRequirement {
        name: name.to_owned(),
        version: Version::new(
            comparator.major,
            comparator.minor.unwrap_or(0),
            comparator.patch.unwrap_or(0),
        ),
    })
}

fn minimum_edges(
    requirements: &BTreeMap<String, MinimumRequirement>,
    selected: &BTreeMap<(String, u64), &crate::Release>,
) -> BTreeMap<String, String> {
    requirements
        .iter()
        .map(|(alias, requirement)| {
            let release = selected
                .get(&requirement.identity())
                .expect("every visited minimum has a selected release");
            assert!(release.version >= requirement.version);
            (alias.clone(), release.key())
        })
        .collect()
}
