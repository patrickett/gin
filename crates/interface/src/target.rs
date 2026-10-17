use crate::Fingerprint;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetProfile {
    pub target: String,
}

impl TargetProfile {
    pub(crate) fn sort_key(&self) -> &str {
        &self.target
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetRealization {
    pub target: String,
    pub semantic_surface_fingerprint: Fingerprint,
    pub public_closure_fingerprint: Fingerprint,
}
