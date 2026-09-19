use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{Registry, Release, ReleaseStatus, SecurityEvent, SecurityPolicy};

const DOMAIN: &[u8] = b"flasks.snapshot.v1\0";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustRoot {
    pub registry: String,
    pub keys: BTreeMap<String, String>,
    pub threshold: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub registry: String,
    pub revision: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub releases: Vec<Release>,
    pub events: Vec<SecurityEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSignature {
    pub key_id: String,
    pub signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedSnapshot {
    pub payload: String,
    pub signatures: Vec<SnapshotSignature>,
}

pub struct VerifiedSnapshot {
    snapshot: Snapshot,
    digest: [u8; 32],
    registry: Registry,
}

#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("invalid metadata: {0}")]
    Invalid(String),
    #[error("registry metadata does not meet the pinned signature threshold")]
    Signature,
    #[error("registry metadata has expired or is dated in the future")]
    Time,
    #[error("registry metadata rollback or equivocation detected")]
    Rollback,
    #[error("registry attempted to rewrite release or security history")]
    History,
}

impl SignedSnapshot {
    pub fn sign(
        snapshot: &Snapshot,
        keys: &BTreeMap<String, SigningKey>,
    ) -> Result<Self, MetadataError> {
        let payload =
            serde_json::to_string(snapshot).map_err(|e| MetadataError::Invalid(e.to_string()))?;
        let message = [DOMAIN, payload.as_bytes()].concat();
        let signatures = keys
            .iter()
            .map(|(key_id, key)| SnapshotSignature {
                key_id: key_id.clone(),
                signature: hex(&key.sign(&message).to_bytes()),
            })
            .collect();
        Ok(Self {
            payload,
            signatures,
        })
    }

    pub fn verify(
        &self,
        trust: &TrustRoot,
        now: u64,
        previous: Option<&VerifiedSnapshot>,
    ) -> Result<VerifiedSnapshot, MetadataError> {
        let verified = self.authenticate(trust)?;
        if verified.snapshot.issued_at > now || verified.snapshot.expires_at <= now {
            return Err(MetadataError::Time);
        }
        if let Some(previous) = previous {
            verified.check_history(previous)?;
        }
        Ok(verified)
    }

    pub fn authenticate(&self, trust: &TrustRoot) -> Result<VerifiedSnapshot, MetadataError> {
        if self.payload.len() > 16 * 1024 * 1024
            || self.signatures.len() > 32
            || trust.threshold == 0
            || trust.threshold > trust.keys.len()
            || trust.keys.len() > 32
        {
            return Err(MetadataError::Signature);
        }
        let message = [DOMAIN, self.payload.as_bytes()].concat();
        let mut accepted = BTreeSet::new();
        for sig in &self.signatures {
            let Some(key) = trust.keys.get(&sig.key_id) else {
                continue;
            };
            let key = VerifyingKey::from_bytes(&unhex::<32>(key)?)
                .map_err(|_| MetadataError::Signature)?;
            let signature = Signature::from_bytes(&unhex::<64>(&sig.signature)?);
            if key.verify_strict(&message, &signature).is_ok() {
                accepted.insert(key.to_bytes());
            }
        }
        if accepted.len() < trust.threshold {
            return Err(MetadataError::Signature);
        }
        let snapshot: Snapshot = serde_json::from_str(&self.payload)
            .map_err(|e| MetadataError::Invalid(e.to_string()))?;
        if snapshot.schema != 1
            || snapshot.registry != trust.registry
            || snapshot.revision == 0
            || snapshot.expires_at <= snapshot.issued_at
            || snapshot.expires_at - snapshot.issued_at > 7 * 86400
        {
            return Err(MetadataError::Invalid(
                "unsupported schema, registry, revision, or validity period".into(),
            ));
        }
        let registry = Registry::from_snapshot(&snapshot)?;
        Ok(VerifiedSnapshot {
            snapshot,
            registry,
            digest: Sha256::digest(self.payload.as_bytes()).into(),
        })
    }
}

impl VerifiedSnapshot {
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn policy(&self, now: u64, cooldown_seconds: u64) -> Result<SecurityPolicy, MetadataError> {
        if now < self.snapshot.issued_at || now >= self.snapshot.expires_at {
            return Err(MetadataError::Time);
        }
        Ok(SecurityPolicy {
            now,
            metadata_expires_at: self.snapshot.expires_at,
            cooldown_seconds,
        })
    }

    fn check_history(&self, old: &Self) -> Result<(), MetadataError> {
        if self.snapshot.registry != old.snapshot.registry
            || self.snapshot.revision < old.snapshot.revision
            || (self.snapshot.revision == old.snapshot.revision && self.digest != old.digest)
            || self.snapshot.issued_at < old.snapshot.issued_at
        {
            return Err(MetadataError::Rollback);
        }
        if !self.snapshot.events.starts_with(&old.snapshot.events) {
            return Err(MetadataError::History);
        }
        for release in &old.snapshot.releases {
            if self.registry.release(&release.name, &release.version).ok() != Some(release) {
                return Err(MetadataError::History);
            }
        }
        Ok(())
    }
}

impl Registry {
    pub fn snapshot(
        &self,
        registry: String,
        revision: u64,
        issued_at: u64,
        expires_at: u64,
    ) -> Snapshot {
        Snapshot {
            schema: 1,
            registry,
            revision,
            issued_at,
            expires_at,
            releases: self.releases().cloned().collect(),
            events: self.security_events().to_vec(),
        }
    }

    fn from_snapshot(snapshot: &Snapshot) -> Result<Self, MetadataError> {
        let mut registry = Self::default();
        for release in &snapshot.releases {
            if release.published_at > snapshot.issued_at {
                return Err(MetadataError::Time);
            }
            registry
                .insert(release.clone())
                .map_err(|e| MetadataError::Invalid(e.to_string()))?;
        }
        let known: BTreeMap<_, _> = snapshot.releases.iter().map(|r| (r.key(), r)).collect();
        let mut seen = BTreeSet::new();
        for (i, event) in snapshot.events.iter().enumerate() {
            let release = known.get(&event.release).ok_or(MetadataError::History)?;
            if event.revision != i as u64 + 1
                || (seen.insert(&event.release) && event.status != ReleaseStatus::Quarantined)
            {
                return Err(MetadataError::History);
            }
            registry
                .set_status(&release.name, &release.version, event.status.clone())
                .map_err(|_| MetadataError::History)?;
        }
        if seen.len() != known.len() {
            return Err(MetadataError::History);
        }
        registry.replace_events(snapshot.events.clone());
        Ok(registry)
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Result<[u8; N], MetadataError> {
    if s.len() != N * 2 || !s.is_ascii() {
        return Err(MetadataError::Signature);
    }
    let mut bytes = [0u8; N];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte =
            u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| MetadataError::Signature)?;
    }
    Ok(bytes)
}
