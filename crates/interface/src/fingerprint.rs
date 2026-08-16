use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Self(hasher.finalize().into())
    }

    pub fn from_domain(domain: &[u8], schema_version: u32, payload: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(domain);
        hasher.update(schema_version.to_le_bytes());
        hasher.update(payload);
        Self(hasher.finalize().into())
    }

    pub fn to_hex(self) -> String {
        self.0
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    }
}
