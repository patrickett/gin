use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path};
use std::time::Duration;

use crate::{PackageResolution, Release, SignedSnapshot, TrustRoot, VerifiedSnapshot};
use serde::{Deserialize, Serialize};

use crate::{DependencyKind, FlaskConfig, GinHome, PACKAGE_CONFIG_NAME};

const METADATA_LIMIT: usize = 32 * 1024 * 1024;
const ARCHIVE_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrySettings {
    pub url: String,
    pub trust: TrustRoot,
    #[serde(default = "cooldown")]
    pub cooldown_seconds: u64,
}

fn cooldown() -> u64 {
    3 * 86400
}
fn error(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

pub trait RegistryTransport {
    fn fetch(&self, resource: &str, limit: usize) -> io::Result<Vec<u8>>;
}

pub struct HttpsTransport {
    url: String,
    agent: ureq::Agent,
}

impl HttpsTransport {
    pub fn new(url: &str) -> io::Result<Self> {
        let uri: ureq::http::Uri = url.parse().map_err(error)?;
        if uri.scheme_str() != Some("https")
            || uri.authority().is_none()
            || uri.authority().is_some_and(|a| a.as_str().contains('@'))
            || uri.query().is_some()
            || uri.path() != "/"
        {
            return Err(error(
                "registry URL must be an HTTPS origin without credentials, query, or path",
            ));
        }
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Ok(Self {
            url: url.trim_end_matches('/').to_owned(),
            agent: config.into(),
        })
    }
}

impl RegistryTransport for HttpsTransport {
    fn fetch(&self, resource: &str, limit: usize) -> io::Result<Vec<u8>> {
        let mut response = self
            .agent
            .get(format!("{}/{}", self.url, resource))
            .call()
            .map_err(error)?;
        if response.status() != 200 {
            return Err(error("registry returned an unexpected HTTP status"));
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(error("registry response exceeds size limit"));
        }
        Ok(bytes)
    }
}

pub struct RegistryClient<T> {
    home: GinHome,
    settings: RegistrySettings,
    transport: T,
}

impl RegistryClient<HttpsTransport> {
    pub fn load(home: GinHome) -> io::Result<Self> {
        let settings = home.root().join("registry.json");
        let settings: RegistrySettings =
            serde_json::from_slice(&fs::read(&settings).map_err(|e| {
                error(format!(
                    "cannot read registry trust configuration {}: {e}",
                    settings.display()
                ))
            })?)
            .map_err(error)?;
        let transport = HttpsTransport::new(&settings.url)?;
        Ok(Self::new(home, settings, transport))
    }
}

impl<T: RegistryTransport> RegistryClient<T> {
    pub fn registry_name(&self) -> &str {
        &self.settings.trust.registry
    }
    pub fn new(home: GinHome, settings: RegistrySettings, transport: T) -> Self {
        Self {
            home,
            settings,
            transport,
        }
    }

    pub fn metadata(&self, now: u64, refresh: bool) -> io::Result<VerifiedSnapshot> {
        let security = self.home.root().join("security");
        fs::create_dir_all(&security)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(security.join("registry.lock"))?;
        lock.lock()?;
        let path = security.join("snapshot.json");
        let previous = match read_bounded(&path, METADATA_LIMIT) {
            Ok(bytes) => {
                if bytes.len() > METADATA_LIMIT {
                    return Err(error("cached metadata exceeds size limit"));
                }
                let signed: SignedSnapshot = serde_json::from_slice(&bytes).map_err(error)?;
                Some(signed.authenticate(&self.settings.trust).map_err(error)?)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if !refresh {
            let previous = previous
                .ok_or_else(|| error("no authenticated metadata; run begin fetch first"))?;
            previous
                .policy(now, self.settings.cooldown_seconds)
                .map_err(error)?;
            return Ok(previous);
        }
        let bytes = self.transport.fetch("v1/snapshot.json", METADATA_LIMIT)?;
        if bytes.len() > METADATA_LIMIT {
            return Err(error("metadata exceeds size limit"));
        }
        let signed: SignedSnapshot = serde_json::from_slice(&bytes).map_err(error)?;
        let verified = signed
            .verify(&self.settings.trust, now, previous.as_ref())
            .map_err(error)?;
        let mut staged = tempfile::NamedTempFile::new_in(&security)?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        staged.persist(&path).map_err(|e| e.error)?;
        Ok(verified)
    }

    pub fn resolve(&self, config: &FlaskConfig, now: u64) -> io::Result<PackageResolution> {
        let metadata = self.metadata(now, true)?;
        metadata
            .registry()
            .resolve(
                &config.registry_requirements().map_err(error)?,
                metadata
                    .policy(now, self.settings.cooldown_seconds)
                    .map_err(error)?,
            )
            .map_err(error)
    }

    pub fn audit(&self, config: &FlaskConfig, now: u64) -> io::Result<()> {
        let resolution = config
            .resolution
            .as_ref()
            .ok_or_else(|| error("manifest has no resolution; run begin update"))?;
        let metadata = self.metadata(now, true)?;
        metadata
            .registry()
            .validate_resolution(
                &config.registry_requirements().map_err(error)?,
                resolution,
                metadata
                    .policy(now, self.settings.cooldown_seconds)
                    .map_err(error)?,
            )
            .map_err(error)
    }

    pub fn fetch(&self, config: &FlaskConfig, now: u64, refresh: bool) -> io::Result<()> {
        let resolution = config
            .resolution
            .as_ref()
            .ok_or_else(|| error("manifest has no resolution; run begin update first"))?;
        let metadata = self.metadata(now, refresh)?;
        metadata
            .registry()
            .validate_resolution(
                &config.registry_requirements().map_err(error)?,
                resolution,
                metadata
                    .policy(now, self.settings.cooldown_seconds)
                    .map_err(error)?,
            )
            .map_err(error)?;
        for package in resolution.packages.values() {
            let release = metadata
                .registry()
                .release(&package.name, &package.version)
                .map_err(error)?;
            self.install(release)?;
        }
        Ok(())
    }

    pub fn verify_cached(&self, config: &FlaskConfig, now: u64) -> io::Result<PackageResolution> {
        let resolution = config
            .resolution
            .as_ref()
            .ok_or_else(|| error("manifest has no exact resolution; run begin update"))?;
        let metadata = self.metadata(now, false)?;
        metadata
            .registry()
            .validate_resolution(
                &config.registry_requirements().map_err(error)?,
                resolution,
                metadata
                    .policy(now, self.settings.cooldown_seconds)
                    .map_err(error)?,
            )
            .map_err(error)?;
        for package in resolution.packages.values() {
            let release = metadata
                .registry()
                .release(&package.name, &package.version)
                .map_err(error)?;
            let archive = self.archive(release)?;
            let entries = release_files(release, &archive)?;
            let destination = self.home.dependency(&release.name, &release.version)?;
            verify_tree(&destination, &entries)?;
        }
        Ok(resolution.clone())
    }

    pub fn interface(
        &self,
        name: &str,
        version: &semver::Version,
        now: u64,
    ) -> io::Result<Vec<u8>> {
        let metadata = self.metadata(now, true)?;
        let release = metadata.registry().release(name, version).map_err(error)?;
        metadata
            .registry()
            .check(
                release,
                metadata
                    .policy(now, self.settings.cooldown_seconds)
                    .map_err(error)?,
            )
            .map_err(error)?;
        let bytes = self.transport.fetch(
            &format!("v1/blobs/{}", release.interface_sha256),
            ARCHIVE_LIMIT,
        )?;
        use sha2::{Digest, Sha256};
        if bytes.len() > ARCHIVE_LIMIT
            || format!("{:x}", Sha256::digest(&bytes)) != release.interface_sha256
        {
            return Err(error("interface digest mismatch"));
        }
        Ok(bytes)
    }

    fn archive(&self, release: &Release) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(
            self.home
                .root()
                .join("cache/archives")
                .join(&release.archive_sha256),
        )?
        .take(ARCHIVE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
        if bytes.len() > ARCHIVE_LIMIT {
            return Err(error("archive exceeds size limit"));
        }
        release.verify_archive(&bytes).map_err(error)?;
        Ok(bytes)
    }

    fn install(&self, release: &Release) -> io::Result<()> {
        let deps = self.home.root().join("cache/deps");
        fs::create_dir_all(&deps)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(deps.join(".install.lock"))?;
        lock.lock()?;
        let archive = match self.archive(release) {
            Ok(bytes) => bytes,
            Err(_) => {
                let bytes = self.transport.fetch(
                    &format!("v1/blobs/{}", release.archive_sha256),
                    ARCHIVE_LIMIT,
                )?;
                if bytes.len() > ARCHIVE_LIMIT {
                    return Err(error("archive exceeds size limit"));
                }
                release.verify_archive(&bytes).map_err(error)?;
                let archives = self.home.root().join("cache/archives");
                fs::create_dir_all(&archives)?;
                let mut staged = tempfile::NamedTempFile::new_in(&archives)?;
                staged.write_all(&bytes)?;
                staged
                    .persist(archives.join(&release.archive_sha256))
                    .map_err(|e| e.error)?;
                bytes
            }
        };
        let entries = release_files(release, &archive)?;
        let destination = self.home.dependency(&release.name, &release.version)?;
        if destination.exists() {
            return verify_tree(&destination, &entries);
        }
        let staging = tempfile::tempdir_in(&deps)?;
        for (path, bytes) in entries {
            let path = staging.path().join(path);
            fs::create_dir_all(path.parent().ok_or_else(|| error("invalid archive path"))?)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(&bytes)?;
        }
        fs::rename(staging.path(), &destination)?;
        Ok(())
    }
}

fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(error("cached file exceeds size limit"));
    }
    Ok(bytes)
}

fn release_files(release: &Release, archive: &[u8]) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let entries = archive_files(archive)?;
    let raw = entries
        .get(PACKAGE_CONFIG_NAME)
        .ok_or_else(|| error("archive has no root flask.jsonc"))?;
    let manifest: FlaskConfig =
        json5::from_str(std::str::from_utf8(raw).map_err(error)?).map_err(error)?;
    if manifest.name != release.name
        || manifest.version != release.version.to_string()
        || manifest.registry_requirements().map_err(error)? != release.dependencies
        || manifest
            .dependencies()
            .values()
            .any(|d| !matches!(d.kind, DependencyKind::Version { .. }))
    {
        return Err(error(
            "archive manifest does not match signed release metadata",
        ));
    }
    Ok(entries)
}

fn archive_files(bytes: &[u8]) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let mut archive = tar::Archive::new(bytes);
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for (i, entry) in archive.entries()?.enumerate() {
        if i >= 10_000 {
            return Err(error("archive has too many entries"));
        }
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(error("archive links and special files are forbidden"));
        }
        let path = entry.path()?.into_owned();
        let text = path
            .to_str()
            .ok_or_else(|| error("archive path is not UTF-8"))?;
        let text = if kind.is_dir() {
            text.trim_end_matches('/')
        } else {
            text
        };
        if text.is_empty()
            || text.contains(['\\', ':'])
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || text
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(error("archive path escapes or aliases its package root"));
        }
        if kind.is_dir() {
            continue;
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| error("archive size overflow"))?;
        if total > ARCHIVE_LIMIT as u64 {
            return Err(error("expanded archive exceeds size limit"));
        }
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents)?;
        if files.insert(text.to_owned(), contents).is_some() {
            return Err(error("duplicate archive file"));
        }
    }
    Ok(files)
}

fn verify_tree(root: &Path, files: &BTreeMap<String, Vec<u8>>) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(dir) = pending.pop() {
        if !fs::symlink_metadata(&dir)?.file_type().is_dir() {
            return Err(error("cached package directory is not a regular directory"));
        }
        for entry in fs::read_dir(&dir)? {
            visited += 1;
            if visited > 20_000 {
                return Err(error("cached dependency exceeds entry limit"));
            }
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                let path = entry.path();
                let name = path
                    .strip_prefix(root)
                    .map_err(error)?
                    .to_str()
                    .ok_or_else(|| error("invalid cached path"))?
                    .to_owned();
                let expected = files
                    .get(&name)
                    .ok_or_else(|| error("unexpected file in cached dependency"))?;
                let mut actual = Vec::new();
                File::open(&path)?
                    .take(expected.len() as u64 + 1)
                    .read_to_end(&mut actual)?;
                if &actual != expected {
                    return Err(error("cached dependency content has changed"));
                }
                seen.insert(name);
            } else {
                return Err(error("cached dependency contains a link or special file"));
            }
        }
        if pending.len() + seen.len() > 20_000 {
            return Err(error("cached dependency exceeds entry limit"));
        }
    }
    if seen.len() != files.len() {
        return Err(error("cached dependency is incomplete"));
    }
    Ok(())
}
