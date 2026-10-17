use crate::Fingerprint;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComptimeImage {
    pub target: String,
    pub image_fingerprint: Fingerprint,
}
