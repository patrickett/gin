use super::InterfaceSignature;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// On-disk metadata stored alongside a cached object file (`manifest.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheManifest {
    /// Schema version (starts at 1)
    pub version: u32,
    /// Full SHA-256 hex of the raw source text
    pub content_hash: String,
    /// SHA-256 hex of the public API surface
    pub interface_hash: String,
    /// Target triple
    pub target: String,
    /// Build profile ("debug" or "release")
    pub profile: String,
    /// Original source file paths
    pub source_paths: Vec<PathBuf>,
    /// Dependency name -> interface hash at compile time
    pub dependency_interfaces: HashMap<String, String>,
    /// ISO 8601 timestamp of when this entry was created
    pub created_at: String,
    /// Compiler version that produced this artifact
    pub ginc_version: String,
    /// Serialized public API surface for structural diffing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_signature: Option<InterfaceSignature>,
}

/// Result of looking up a cache entry.
pub enum CacheLookup {
    Hit {
        obj_path: PathBuf,
        manifest: Box<CacheManifest>,
    },
    Miss,
}
