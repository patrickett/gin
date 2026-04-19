use crate::{FlaskConfig, PACKAGE_CONFIG_NAME};
use std::{
    fs::File,
    io::Write,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    NotFound { searched_from: PathBuf },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(err) => write!(f, "IO error: {}", err),
            ConfigError::NotFound { searched_from } => {
                write!(
                    f,
                    "Flask config not found in or above {}",
                    searched_from.display()
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

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

impl FlaskConfigReadGuard<'_> {
    pub fn name(&self) -> &str {
        self.config.name()
    }

    pub fn dependency_names(&self) -> Vec<&str> {
        self.config.dependency_names()
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
        let mut search = from_dir.to_path_buf();
        loop {
            search.push(PACKAGE_CONFIG_NAME);
            match std::fs::File::open(&search) {
                Ok(file) => {
                    let config =
                        serde_json::from_reader::<_, FlaskConfig>(file).map_err(|_err| {
                            ConfigError::Io(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "failed to parse config",
                            ))
                        })?;
                    search.pop();
                    return Ok(Self {
                        inner: Arc::new(RwLock::new(FlaskConfigHandleInner {
                            config,
                            source_dir: search.clone(),
                        })),
                    });
                }
                Err(_) => {
                    search.pop();
                    if !search.pop() {
                        return Err(ConfigError::NotFound {
                            searched_from: from_dir.to_path_buf(),
                        });
                    }
                }
            }
        }
    }

    pub fn read(&self) -> FlaskConfigReadGuard<'_> {
        FlaskConfigReadGuard {
            guard: self.inner.read().unwrap(),
        }
    }

    pub fn write(&self) -> FlaskConfigWriteGuard<'_> {
        FlaskConfigWriteGuard {
            guard: self.inner.write().unwrap(),
        }
    }

    pub fn source_dir(&self) -> PathBuf {
        self.inner.read().unwrap().source_dir.clone()
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let config = self.read().config.clone();
        let json = serde_json::to_string_pretty(&config).expect("failed to serialize config");
        let source_dir = self.source_dir();
        let mut file =
            File::create(source_dir.join(PACKAGE_CONFIG_NAME)).map_err(ConfigError::Io)?;
        file.write_all(json.as_bytes()).map_err(ConfigError::Io)?;
        Ok(())
    }

    pub fn set_name(&mut self, name: String) {
        let mut inner = self.write();
        inner.config.set_name(name);
    }

    pub fn set_version(&mut self, version: String) {
        let mut inner = self.write();
        inner.config.set_version(version);
    }
}
