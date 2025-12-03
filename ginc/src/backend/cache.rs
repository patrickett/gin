use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::{fs, path::PathBuf};

// pub enum Cache<T> {
//     Hit(T),
//     Miss,
// }

impl From<&Path> for PackageId {
    fn from(val: &Path) -> Self {
        let mut hasher = Sha256::new();
        let s = val
            .as_os_str()
            .to_str()
            .expect("failed to create packageid");
        hasher.update(s.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        PackageId(hash)
    }
}

impl PackageId {
    pub fn new(path: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(path.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        Self(hash)
    }
}

/// Represents compiled output (could be lib, bin, or both).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompiledOutput {
    Library(Vec<u8>), // pretend object file
    BinaryAndLibrary { lib: Vec<u8>, bin: Vec<u8> },
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct PackageId(pub String); // fingerprint of package contents

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub package_id: PackageId,
    pub dep_ids: Vec<PackageId>, // edges in the dependency graph
    pub output: CompiledOutput,
}

/// Global cache of compiled packages, persisted on disk.
#[derive(Debug)]
pub struct GlobalCache {
    cache_dir: PathBuf,
    entries: HashMap<PackageId, CacheEntry>,
}

pub const CACHE_DIR: &str = ".gin_cache";

impl Default for GlobalCache {
    fn default() -> Self {
        Self::new(PathBuf::from(CACHE_DIR))
    }
}

impl GlobalCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        fs::create_dir_all(&cache_dir).unwrap();
        let mut entries = HashMap::new();

        // load all .json cache files into memory
        for entry in fs::read_dir(&cache_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().and_then(|s| s.to_str()) == Some("json")
                && let Ok(text) = fs::read_to_string(entry.path())
                && let Ok(e) = serde_json::from_str::<CacheEntry>(&text)
            {
                entries.insert(e.package_id.clone(), e);
            }
        }

        Self { cache_dir, entries }
    }

    pub fn get(&self, id: &PackageId) -> Option<&CompiledOutput> {
        self.entries.get(id).map(|e| &e.output)
    }

    pub fn insert(&mut self, entry: CacheEntry) {
        let id = entry.package_id.clone();
        let path = self.cache_dir.join(format!("{}.json", &id.0));
        fs::write(path, serde_json::to_string(&entry).unwrap()).unwrap();
        self.entries.insert(id, entry);
    }

    /// Perform dependency-reachability GC.
    /// Keep only entries reachable from `roots`.
    pub fn gc(&mut self, roots: &[PackageId]) {
        let mut reachable = HashSet::new();
        let mut stack = roots.to_vec();

        while let Some(id) = stack.pop() {
            if reachable.insert(id.clone())
                && let Some(entry) = self.entries.get(&id)
            {
                for dep in &entry.dep_ids {
                    stack.push(dep.clone());
                }
            }
        }

        // Remove entries not in reachable set
        self.entries.retain(|id, _| {
            if reachable.contains(id) {
                true
            } else {
                let path = self.cache_dir.join(format!("{}.json", &id.0));
                let _ = fs::remove_file(path);
                false
            }
        });
    }
}
