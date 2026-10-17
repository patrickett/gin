use derive_more::Display;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Feature {}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct DependencyCommon {
    #[serde(default)]
    pub features: Vec<Feature>,
    #[serde(default)]
    pub optional: bool,

    /// TODO:
    /// Automatically insert at the top of all files.
    #[serde(default)]
    pub auto: bool,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct BugInfo {
    #[serde(default)]
    pub(crate) url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
#[serde(untagged)]
pub enum DependencyKind {
    Version { version: String },
    Path { path: String },
    Git { url: String },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Dependency {
    #[serde(flatten)]
    pub kind: DependencyKind,

    #[serde(flatten)]
    pub common: DependencyCommon,
}

impl Dependency {
    pub fn new(kind: DependencyKind) -> Self {
        Self {
            kind,
            common: Default::default(),
        }
    }

    pub fn minimum(alias: &str, requirement: &crate::MinimumRequirement) -> Self {
        Self::new(DependencyKind::Version {
            version: requirement.manifest_value(alias),
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Display)]
#[display("{_0}")]
pub struct Author(pub String);

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct FlaskConfig {
    pub name: String,
    pub description: Option<String>,
    pub version: String,
    #[serde(default)]
    pub(crate) keywords: Option<Vec<String>>,
    pub(crate) authors: Vec<Author>,
    #[serde(default)]
    pub(crate) repository: Option<String>,
    #[serde(default)]
    pub(crate) license: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) bugs: Option<BugInfo>,
    #[serde(default)]
    pub(crate) funding: Option<Vec<String>>,
    /// `"library"` or full target triple (`arch-vendor-os`).
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub(crate) dependencies: HashMap<String, Dependency>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub minimum_version_selection: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<crate::PackageResolution>,
}

impl FlaskConfig {
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            description: None,
            version,
            keywords: None,
            authors: vec![],
            repository: None,
            license: None,
            bugs: None,
            funding: None,
            target: None,
            dependencies: HashMap::new(),
            minimum_version_selection: false,
            resolution: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn authors(&self) -> &[Author] {
        &self.authors
    }

    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub fn dependency_names(&self) -> Vec<&str> {
        self.dependencies.keys().map(|s| s.as_str()).collect()
    }

    pub fn dependencies(&self) -> &HashMap<String, Dependency> {
        &self.dependencies
    }

    /// Absolute roots for every `Path` dependency, resolved against `base`.
    pub fn resolve_path_dependencies(&self, base: &Path) -> HashMap<String, PathBuf> {
        let mut dependencies = HashMap::new();
        for (name, dependency) in &self.dependencies {
            if let DependencyKind::Path { path } = &dependency.kind {
                dependencies.insert(name.clone(), base.join(path));
            }
        }
        dependencies
    }

    /// Find the package configuration and its root directory by walking up from `dir`.
    /// Returns `Some((config, root_dir))` when a `mod.flask` is found.
    pub fn find_package_config(dir: &std::path::Path) -> Option<(FlaskConfig, std::path::PathBuf)> {
        Self::discover(dir).ok().flatten()
    }

    pub fn discover(
        from: &std::path::Path,
    ) -> Result<Option<(FlaskConfig, std::path::PathBuf)>, String> {
        for root in from.ancestors() {
            let path = Self::manifest_path(root);
            match std::fs::read_to_string(&path) {
                Ok(raw) => {
                    return Self::parse_manifest(&raw)
                        .map(|config| Some((config, root.to_path_buf())))
                        .map_err(|error| format!("{}: {error}", path.display()));
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) => {}
                Err(error) => return Err(format!("{}: {error}", path.display())),
            }
        }
        Ok(None)
    }

    pub fn registry_minimum_requirements(
        &self,
    ) -> Result<std::collections::BTreeMap<String, crate::MinimumRequirement>, crate::ResolutionError>
    {
        self.version_deps()
            .map(|(alias, version)| {
                crate::MinimumRequirement::parse(alias, version)
                    .map(|requirement| (alias.clone(), requirement))
            })
            .collect()
    }

    pub fn with_minimum_requirement(
        &self,
        alias: &str,
        requirement: crate::MinimumRequirement,
    ) -> Result<Self, crate::ResolutionError> {
        if !self.minimum_version_selection || !crate::valid_package_name(alias) {
            return Err(crate::ResolutionError(
                "minimum dependencies require mod.flask and a valid alias".into(),
            ));
        }
        requirement.validate()?;
        let mut next = self.clone();
        next.dependencies
            .insert(alias.to_owned(), Dependency::minimum(alias, &requirement));
        Ok(next)
    }

    pub fn with_minimum_floors(
        &self,
        resolution: &crate::PackageResolution,
    ) -> Result<Self, crate::ResolutionError> {
        let mut result = self.clone();
        let requirements = self.registry_minimum_requirements()?;
        for package in resolution.packages.values() {
            let alias = requirements
                .iter()
                .find(|(_, r)| r.name == package.name && r.version.major == package.version.major)
                .map(|(alias, _)| alias.clone())
                .unwrap_or_else(|| {
                    if result.dependencies.contains_key(&package.name) {
                        format!("{}_v{}", package.name, package.version.major)
                    } else {
                        package.name.clone()
                    }
                });
            if let Some(dependency) = result.dependencies.get(&alias) {
                let DependencyKind::Version { version } = &dependency.kind else {
                    return Err(crate::ResolutionError(format!(
                        "dependency alias collision: {alias}"
                    )));
                };
                let existing = crate::MinimumRequirement::parse(&alias, version)?;
                if existing.name != package.name || existing.version.major != package.version.major
                {
                    return Err(crate::ResolutionError(format!(
                        "dependency alias collision: {alias}"
                    )));
                }
            }
            let minimum = crate::MinimumRequirement {
                name: package.name.clone(),
                version: package.version.clone(),
            };
            result
                .dependencies
                .insert(alias.clone(), Dependency::minimum(&alias, &minimum));
        }
        Ok(result)
    }

    fn version_deps(&self) -> impl Iterator<Item = (&String, &String)> {
        self.dependencies
            .iter()
            .filter_map(|(alias, dependency)| match &dependency.kind {
                DependencyKind::Version { version } => Some((alias, version)),
                _ => None,
            })
    }

    pub fn registry_requirements(
        &self,
    ) -> Result<std::collections::BTreeMap<String, semver::VersionReq>, crate::ResolutionError>
    {
        self.version_deps()
            .map(|(name, version)| {
                semver::VersionReq::parse(version)
                    .map(|req| (name.clone(), req))
                    .map_err(|e| crate::ResolutionError(e.to_string()))
            })
            .collect()
    }

    pub fn validate_resolution(&self) -> Result<(), crate::ResolutionError> {
        if self.minimum_version_selection {
            if self.resolution.is_some() {
                return Err(crate::ResolutionError(
                    "mod.flask cannot contain a locked resolution".into(),
                ));
            }
            self.registry_minimum_requirements()?;
            return Ok(());
        }
        if let Some(resolution) = &self.resolution {
            resolution.validate(&self.registry_requirements()?)?;
        }
        Ok(())
    }
}
