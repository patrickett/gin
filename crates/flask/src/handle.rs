use crate::{FlaskConfig, PACKAGE_CONFIG_NAME};
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
        self.update_field("version", serde_json::Value::String(next.to_owned()))
    }

    pub fn update_resolution(
        &self,
        resolution: crate::PackageResolution,
    ) -> Result<(), ConfigError> {
        resolution
            .validate(
                &self
                    .read()
                    .config
                    .registry_requirements()
                    .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?,
            )
            .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
        self.update_field(
            "resolution",
            serde_json::to_value(resolution)
                .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?,
        )
    }

    fn update_field(&self, field: &str, value: serde_json::Value) -> Result<(), ConfigError> {
        let path = self.source_dir().join(PACKAGE_CONFIG_NAME);
        let raw = std::fs::read_to_string(&path).map_err(ConfigError::Io)?;
        let config: FlaskConfig = json5::from_str(&raw).map_err(|e| {
            ConfigError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;
        let encode = |config: &FlaskConfig| {
            serde_json::to_value(config).map_err(|e| ConfigError::Io(std::io::Error::other(e)))
        };
        if encode(&config)? != encode(&self.read().config)? {
            return Err(ConfigError::Io(std::io::Error::other(
                "manifest changed during operation",
            )));
        }
        let encoded = serde_json::to_string_pretty(&value)
            .map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
        let updated = crate::manifest_edit::set_field(&raw, field, &encoded).ok_or_else(|| {
            ConfigError::Io(std::io::Error::other("cannot update manifest field"))
        })?;
        let next: FlaskConfig =
            json5::from_str(&updated).map_err(|e| ConfigError::Io(std::io::Error::other(e)))?;
        let mut tmp =
            tempfile::NamedTempFile::new_in(self.source_dir()).map_err(ConfigError::Io)?;
        tmp.write_all(updated.as_bytes()).map_err(ConfigError::Io)?;
        tmp.as_file()
            .set_permissions(
                std::fs::metadata(&path)
                    .map_err(ConfigError::Io)?
                    .permissions(),
            )
            .map_err(ConfigError::Io)?;
        if std::fs::read_to_string(&path).map_err(ConfigError::Io)? != raw {
            return Err(ConfigError::Io(std::io::Error::other(
                "manifest changed during operation",
            )));
        }
        tmp.persist(&path).map_err(|e| ConfigError::Io(e.error))?;
        self.write().config = next;
        Ok(())
    }

    /// Load config from a directory, searching upward for flask.jsonc.
    pub fn load(from_dir: &Path) -> Result<Self, ConfigError> {
        let (config, source_dir) =
            FlaskConfig::find_package_config(from_dir).ok_or_else(|| ConfigError::NotFound {
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
        let json = json5::to_string(&config).map_err(|_| {
            ConfigError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "failed to serialize config",
            ))
        })?;
        let source_dir = self.source_dir();
        let mut file =
            File::create(source_dir.join(PACKAGE_CONFIG_NAME)).map_err(ConfigError::Io)?;
        file.write_all(json.as_bytes()).map_err(ConfigError::Io)?;
        Ok(())
    }
}
