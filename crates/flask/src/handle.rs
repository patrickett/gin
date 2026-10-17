use crate::FlaskConfig;
use std::{
    fs::File,
    io::Write,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(std::io::Error),
    #[error("Flask config not found in or above {searched_from}")]
    NotFound { searched_from: PathBuf },
}

#[derive(Debug, Clone)]
pub struct FlaskConfigHandle {
    inner: Arc<RwLock<FlaskConfigHandleInner>>,
}

#[derive(Debug)]
pub struct FlaskConfigHandleInner {
    pub config: FlaskConfig,
    pub source_dir: PathBuf,
}

#[derive(Debug)]
pub struct FlaskConfigReadGuard<'a> {
    guard: std::sync::RwLockReadGuard<'a, FlaskConfigHandleInner>,
}

impl Deref for FlaskConfigReadGuard<'_> {
    type Target = FlaskConfigHandleInner;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

#[derive(Debug)]
pub struct FlaskConfigWriteGuard<'a> {
    guard: std::sync::RwLockWriteGuard<'a, FlaskConfigHandleInner>,
}

impl Deref for FlaskConfigWriteGuard<'_> {
    type Target = FlaskConfigHandleInner;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl DerefMut for FlaskConfigWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

impl FlaskConfigHandle {
    pub fn update_version(&self, expected: &str, next: &str) -> Result<(), ConfigError> {
        semver::Version::parse(next).map_err(|e| {
            ConfigError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
        })?;
        if self.read().config.version != expected {
            return Err(ConfigError::Io(std::io::Error::other(
                "manifest version changed during compilation",
            )));
        }
        let (path, raw, _) = self.read_unmodified_config()?;
        let updated = crate::manifest::set_entry(&raw, "#settings", "version", next)
            .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
        self.persist_update(&path, &raw, &updated)
    }

    fn persist_update(&self, path: &Path, raw: &str, updated: &str) -> Result<(), ConfigError> {
        let next = FlaskConfig::parse_manifest(updated)
            .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
        let mut tmp =
            tempfile::NamedTempFile::new_in(self.source_dir()).map_err(ConfigError::Io)?;
        tmp.write_all(updated.as_bytes()).map_err(ConfigError::Io)?;
        tmp.as_file()
            .set_permissions(
                std::fs::metadata(path)
                    .map_err(ConfigError::Io)?
                    .permissions(),
            )
            .map_err(ConfigError::Io)?;
        if std::fs::read_to_string(path).map_err(ConfigError::Io)? != raw {
            return Err(ConfigError::Io(std::io::Error::other(
                "manifest changed during operation",
            )));
        }
        tmp.persist(path).map_err(|e| ConfigError::Io(e.error))?;
        self.write().config = next;
        Ok(())
    }

    /// Load config from a directory, searching upward for mod.flask.
    pub fn load(from_dir: &Path) -> Result<Self, ConfigError> {
        let (config, source_dir) = FlaskConfig::discover(from_dir)
            .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?
            .ok_or_else(|| ConfigError::NotFound {
                searched_from: from_dir.to_path_buf(),
            })?;
        Ok(Self {
            inner: Arc::new(RwLock::new(FlaskConfigHandleInner { config, source_dir })),
        })
    }

    pub fn read(&self) -> FlaskConfigReadGuard<'_> {
        FlaskConfigReadGuard {
            guard: self
                .inner
                .read()
                .expect("FlaskConfig RwLock should not be poisoned"),
        }
    }

    pub fn write(&self) -> FlaskConfigWriteGuard<'_> {
        FlaskConfigWriteGuard {
            guard: self
                .inner
                .write()
                .expect("FlaskConfig RwLock should not be poisoned"),
        }
    }

    pub fn source_dir(&self) -> PathBuf {
        self.inner
            .read()
            .expect("FlaskConfig RwLock should not be poisoned")
            .source_dir
            .clone()
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let config = self.read().config.clone();
        let manifest = config
            .format_manifest()
            .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
        let path = FlaskConfig::manifest_path(&self.source_dir());
        if path.exists() {
            let raw = std::fs::read_to_string(&path).map_err(ConfigError::Io)?;
            return self.persist_update(&path, &raw, &manifest);
        }
        let mut file = File::create(path).map_err(ConfigError::Io)?;
        file.write_all(manifest.as_bytes())
            .map_err(ConfigError::Io)?;
        Ok(())
    }

    fn read_unmodified_config(&self) -> Result<(PathBuf, String, FlaskConfig), ConfigError> {
        let path = FlaskConfig::manifest_path(&self.source_dir());
        let raw = std::fs::read_to_string(&path).map_err(ConfigError::Io)?;
        let current = FlaskConfig::parse_manifest(&raw)
            .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
        if serde_json::to_value(&current).unwrap()
            != serde_json::to_value(&self.read().config).unwrap()
        {
            return Err(ConfigError::Io(std::io::Error::other(
                "manifest changed during operation",
            )));
        }
        Ok((path, raw, current))
    }

    pub fn update_minimums(
        &self,
        requirements: &std::collections::BTreeMap<String, crate::MinimumRequirement>,
    ) -> Result<(), ConfigError> {
        let (path, raw, current) = self.read_unmodified_config()?;
        if !current.minimum_version_selection {
            return Err(ConfigError::Io(std::io::Error::other(
                "minimum edits require mod.flask",
            )));
        }
        let mut updated = raw.clone();
        for (alias, requirement) in requirements {
            requirement
                .validate()
                .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
            updated = crate::manifest::set_entry(
                &updated,
                "#dependencies",
                alias,
                &requirement.manifest_value(alias),
            )
            .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
        }
        self.persist_update(&path, &raw, &updated)
    }

    pub fn add_path(&self, alias: &str, dependency: &str) -> Result<(), ConfigError> {
        let (path, raw, _) = self.read_unmodified_config()?;
        let value = serde_json::to_string(dependency).expect("strings serialize");
        let updated = crate::manifest::set_entry(&raw, "#dependencies", alias, &value)
            .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
        self.persist_update(&path, &raw, &updated)
    }
}
