use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::{Dependency, DependencyKind, FlaskConfig, ResolutionError};

impl FlaskConfig {
    pub fn with_workspace_requirements(&self, root: &Path) -> Result<Self, ResolutionError> {
        if !self.minimum_version_selection {
            return Ok(self.clone());
        }
        let root = root
            .canonicalize()
            .map_err(|e| ResolutionError(e.to_string()))?;
        let mut result = self.clone();
        let mut pending = VecDeque::from([(root.clone(), self.clone())]);
        let mut seen = BTreeSet::new();
        let mut identities: BTreeMap<(String, u64), PathBuf> = BTreeMap::new();
        while let Some((path, config)) = pending.pop_front() {
            if !seen.insert(path.clone()) {
                continue;
            }
            if seen.len() > 256 {
                return Err(ResolutionError("too many local packages".into()));
            }
            let version = semver::Version::parse(config.version())
                .map_err(|e| ResolutionError(e.to_string()))?;
            let identity = (config.name().to_owned(), version.major);
            if let Some(previous) = identities.insert(identity, path.clone())
                && previous != path
            {
                return Err(ResolutionError(format!(
                    "conflicting local providers: {} and {}",
                    previous.display(),
                    path.display()
                )));
            }
            if path != root {
                result.merge_workspace_minimums(&config, seen.len())?;
            }
            Self::enqueue_local_packages(&path, &config, &mut pending)?;
        }
        Ok(result)
    }

    fn merge_workspace_minimums(
        &mut self,
        config: &Self,
        index: usize,
    ) -> Result<(), ResolutionError> {
        let minimums = if config.minimum_version_selection {
            config.registry_minimum_requirements()?
        } else {
            config
                .registry_requirements()?
                .iter()
                .map(|(alias, requirement)| {
                    crate::minimum::legacy_minimum(alias, requirement).map(|r| (alias.clone(), r))
                })
                .collect::<Result<_, _>>()?
        };
        for (alias, requirement) in minimums {
            let key = format!("workspace_{index}_{alias}");
            if self.dependencies.contains_key(&key) {
                return Err(ResolutionError(format!(
                    "reserved workspace alias collision: {key}"
                )));
            }
            self.dependencies
                .insert(key.clone(), Dependency::minimum(&key, &requirement));
        }
        Ok(())
    }

    fn enqueue_local_packages(
        path: &Path,
        config: &Self,
        pending: &mut VecDeque<(PathBuf, Self)>,
    ) -> Result<(), ResolutionError> {
        let mut dependencies: Vec<_> = config.dependencies().iter().collect();
        dependencies.sort_unstable_by_key(|(alias, _)| *alias);
        for (_, dependency) in dependencies {
            if let DependencyKind::Path { path: relative } = &dependency.kind {
                let child = match path.join(relative).canonicalize() {
                    Ok(child) => child,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(ResolutionError(error.to_string())),
                };
                let (child_config, found) = Self::discover(&child)
                    .map_err(ResolutionError)?
                    .ok_or_else(|| {
                        ResolutionError(format!(
                            "local package has no manifest: {}",
                            child.display()
                        ))
                    })?;
                if found
                    .canonicalize()
                    .map_err(|e| ResolutionError(e.to_string()))?
                    != child
                {
                    return Err(ResolutionError(format!(
                        "local package has no root manifest: {}",
                        child.display()
                    )));
                }
                if pending.len() >= 4096 {
                    return Err(ResolutionError("too many local dependency edges".into()));
                }
                pending.push_back((child, child_config));
            }
        }
        Ok(())
    }
}
