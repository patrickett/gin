use std::collections::{BTreeMap, BTreeSet};

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageResolution {
    pub direct: BTreeMap<String, String>,
    pub packages: BTreeMap<String, LockedPackage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPackage {
    pub name: String,
    pub version: Version,
    pub sha256: String,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
#[error("invalid dependency resolution: {0}")]
pub struct ResolutionError(pub String);

impl PackageResolution {
    pub fn validate(
        &self,
        requirements: &BTreeMap<String, VersionReq>,
    ) -> Result<(), ResolutionError> {
        let mut expected = BTreeSet::new();
        for (alias, req) in requirements {
            expected.insert(alias.clone());
            let key = self
                .direct
                .get(alias)
                .ok_or_else(|| ResolutionError(format!("missing direct dependency `{alias}`")))?;
            let pkg = self
                .packages
                .get(key)
                .ok_or_else(|| ResolutionError(format!("missing package `{key}`")))?;
            if pkg.name != *alias || !req.matches(&pkg.version) {
                return Err(ResolutionError(format!(
                    "`{key}` does not satisfy `{alias}: {req}`"
                )));
            }
        }
        if expected != self.direct.keys().cloned().collect() {
            return Err(ResolutionError(
                "direct dependencies do not match the manifest".into(),
            ));
        }
        for (key, pkg) in &self.packages {
            if *key != format!("{}@{}", pkg.name, pkg.version)
                || !valid_package_name(&pkg.name)
                || !valid_sha256(&pkg.sha256)
            {
                return Err(ResolutionError(format!(
                    "invalid package identity or checksum `{key}`"
                )));
            }
        }
        let mut visited = BTreeSet::new();
        let mut active = BTreeSet::new();
        for key in self.direct.values() {
            self.visit(key, &mut visited, &mut active)?;
        }
        if visited.len() != self.packages.len() {
            return Err(ResolutionError(
                "resolution contains unreachable packages".into(),
            ));
        }
        Ok(())
    }

    fn visit<'a>(
        &'a self,
        key: &'a str,
        visited: &mut BTreeSet<&'a str>,
        active: &mut BTreeSet<&'a str>,
    ) -> Result<(), ResolutionError> {
        if active.contains(key) {
            return Err(ResolutionError(format!("dependency cycle at `{key}`")));
        }
        if visited.contains(key) {
            return Ok(());
        }
        let pkg = self
            .packages
            .get(key)
            .ok_or_else(|| ResolutionError(format!("missing transitive package `{key}`")))?;
        active.insert(key);
        for child in pkg.dependencies.values() {
            self.visit(child, visited, active)?;
        }
        active.remove(key);
        visited.insert(key);
        Ok(())
    }
}

pub fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

pub(crate) fn valid_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
