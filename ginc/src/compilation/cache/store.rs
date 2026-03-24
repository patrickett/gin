use crate::compilation::cache::{CacheKey, CacheLookup, CacheManifest};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Manages the on-disk compilation cache at `~/.gin/cache/mods/`.
pub struct ModuleCache {
    root: PathBuf,
}

impl ModuleCache {
    /// Create a new `ModuleCache` rooted at `~/.gin/cache/mods/`.
    ///
    /// Returns `None` if the home directory cannot be determined.
    pub fn new() -> Option<Self> {
        let home = dirs::home_dir()?;
        let root = home.join(".gin").join("cache").join("mods");
        Some(Self { root })
    }

    /// Create a `ModuleCache` at a custom root (useful for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    /// Look up a cached artifact by its key.
    ///
    /// Returns `CacheLookup::Hit` if both `module.o` and `manifest.json`
    /// exist and the manifest deserializes successfully.
    pub fn lookup(&self, key: &CacheKey) -> CacheLookup {
        let dir = self.entry_dir(key);
        let obj_path = dir.join("module.o");
        let manifest_path = dir.join("manifest.json");

        if !obj_path.exists() || !manifest_path.exists() {
            return CacheLookup::Miss;
        }

        let manifest_data = match std::fs::read_to_string(&manifest_path) {
            Ok(data) => data,
            Err(_) => return CacheLookup::Miss,
        };

        let manifest: CacheManifest = match serde_json::from_str(&manifest_data) {
            Ok(m) => m,
            Err(_) => return CacheLookup::Miss,
        };

        // Verify full content hash matches (short hash collisions are possible)
        if manifest.content_hash != key.content_hash {
            return CacheLookup::Miss;
        }

        CacheLookup::Hit {
            obj_path,
            manifest: Box::new(manifest),
        }
    }

    /// Store a compiled artifact in the cache.
    ///
    /// Copies the object file from `obj_source` into the cache directory
    /// and writes the manifest alongside it. Returns the path to the
    /// cached object file.
    pub fn store(
        &self,
        key: &CacheKey,
        obj_source: &Path,
        manifest: &CacheManifest,
    ) -> Result<PathBuf, std::io::Error> {
        let dir = self.entry_dir(key);
        std::fs::create_dir_all(&dir)?;

        let obj_dest = dir.join("module.o");
        std::fs::copy(obj_source, &obj_dest)?;

        let manifest_path = dir.join("manifest.json");
        let manifest_json =
            serde_json::to_string_pretty(manifest).map_err(std::io::Error::other)?;
        std::fs::write(&manifest_path, manifest_json)?;

        Ok(obj_dest)
    }

    /// Check whether a cached manifest's dependency interface hashes
    /// still match the current state.
    ///
    /// Returns `true` if every dependency listed in the manifest has the
    /// same interface hash in `current_dep_interfaces`.
    pub fn validate_dependencies(
        manifest: &CacheManifest,
        current_dep_interfaces: &HashMap<String, String>,
    ) -> bool {
        for (dep_name, cached_hash) in &manifest.dependency_interfaces {
            match current_dep_interfaces.get(dep_name) {
                Some(current_hash) if current_hash == cached_hash => continue,
                _ => return false,
            }
        }
        true
    }

    /// Build the directory path for a cache entry.
    fn entry_dir(&self, key: &CacheKey) -> PathBuf {
        self.root
            .join(key.short_hash())
            .join(&key.target)
            .join(&key.profile)
    }

    /// Get the cache root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Build the output directory path for a cache key (public version of entry_dir).
    pub fn output_dir(&self, key: &CacheKey) -> PathBuf {
        self.entry_dir(key)
    }
}
