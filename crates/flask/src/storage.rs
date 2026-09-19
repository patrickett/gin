use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct GinHome(PathBuf);

impl GinHome {
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    pub fn discover() -> io::Result<Self> {
        let path = std::env::var_os("GIN_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".gin")))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot locate Gin home"))?;
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "GIN_HOME must be absolute",
            ));
        }
        Ok(Self(path))
    }

    pub fn root(&self) -> &Path {
        &self.0
    }

    pub fn dependency(&self, name: &str, version: &semver::Version) -> io::Result<PathBuf> {
        if !crate::valid_package_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid registry package name",
            ));
        }
        Ok(self.0.join("cache/deps").join(format!("{name}@{version}")))
    }

    pub fn artifacts(
        &self,
        input: &Path,
        build: &[u8],
        profile: &str,
        target: &str,
    ) -> io::Result<PathBuf> {
        for part in [profile, target] {
            if part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid artifact path component",
                ));
            }
        }
        let input = input.canonicalize()?;
        let identity = Sha256::digest(input.as_os_str().as_encoded_bytes());
        let name = if input.is_dir() {
            input.file_name()
        } else {
            input.file_stem()
        }
        .unwrap_or_default()
        .to_string_lossy();
        let name: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        Ok(self
            .0
            .join("cache/mods")
            .join(format!("{name}-{identity:x}"))
            .join(format!("{:x}", Sha256::digest(build)))
            .join(profile)
            .join(target))
    }
}
