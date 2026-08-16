use crate::{Fingerprint, PackageInstanceId, PublicInterface};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactExpectation {
    pub subject: PackageInstanceId,
    pub digest: Fingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactRecovery {
    Loaded(PublicInterface),
    Rebuilt {
        interface: PublicInterface,
        code: &'static str,
    },
    CacheMiss {
        code: &'static str,
    },
}

pub fn recover_artifact(
    bytes: &[u8],
    expected: &ArtifactExpectation,
    rebuilt: Option<PublicInterface>,
) -> ArtifactRecovery {
    let failure = if Fingerprint::from_bytes(bytes) != expected.digest {
        "interface-artifact-digest-mismatch"
    } else {
        match PublicInterface::decode(bytes) {
            Ok(interface) if interface.subject == expected.subject => {
                return ArtifactRecovery::Loaded(interface);
            }
            Ok(_) => "interface-artifact-key-mismatch",
            Err(_) => "interface-artifact-corrupt",
        }
    };
    rebuilt.map_or(ArtifactRecovery::CacheMiss { code: failure }, |interface| {
        ArtifactRecovery::Rebuilt {
            interface,
            code: failure,
        }
    })
}
