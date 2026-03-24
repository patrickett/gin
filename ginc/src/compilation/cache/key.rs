/// Uniquely identifies a cached compilation artifact.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// SHA-256 hex of the raw source text
    pub content_hash: String,
    /// Target triple (e.g. "aarch64-apple-darwin")
    pub target: String,
    /// Build profile ("debug" or "release")
    pub profile: String,
}

impl CacheKey {
    /// First 16 hex chars of the content hash, used as the directory name.
    pub fn short_hash(&self) -> &str {
        &self.content_hash[..16.min(self.content_hash.len())]
    }
}
