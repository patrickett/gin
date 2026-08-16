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
