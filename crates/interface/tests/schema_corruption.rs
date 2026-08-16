use interface::{
    ArtifactExpectation, ArtifactRecovery, Fingerprint, PackageInstanceId, PublicInterface,
    recover_artifact,
};

fn interface(version: &str) -> PublicInterface {
    PublicInterface::from_subject_and_declarations(
        PackageInstanceId {
            package: "pkg".to_string(),
            version: version.to_string(),
            source: "workspace".to_string(),
            instance: "root".to_string(),
        },
        vec![],
    )
}

#[test]
fn corrupt_artifact_rebuilds_with_source_and_misses_without_source() {
    let valid = interface("1");
    let mut bytes = valid.canonical_bytes();
    let expectation = ArtifactExpectation {
        subject: valid.subject.clone(),
        digest: Fingerprint::from_bytes(&bytes),
    };
    bytes.truncate(4);

    assert!(matches!(
        recover_artifact(&bytes, &expectation, Some(valid.clone())),
        ArtifactRecovery::Rebuilt {
            code: "interface-artifact-digest-mismatch",
            ..
        }
    ));
    assert_eq!(
        recover_artifact(&bytes, &expectation, None),
        ArtifactRecovery::CacheMiss {
            code: "interface-artifact-digest-mismatch"
        }
    );
}

#[test]
fn subject_key_mismatch_is_rejected_even_with_valid_bytes_and_digest() {
    let stored = interface("1");
    let bytes = stored.canonical_bytes();
    let expectation = ArtifactExpectation {
        subject: interface("2").subject,
        digest: Fingerprint::from_bytes(&bytes),
    };

    assert_eq!(
        recover_artifact(&bytes, &expectation, None),
        ArtifactRecovery::CacheMiss {
            code: "interface-artifact-key-mismatch"
        }
    );
}

#[test]
fn valid_artifact_loads_without_rebuild() {
    let stored = interface("1");
    let bytes = stored.canonical_bytes();
    let expectation = ArtifactExpectation {
        subject: stored.subject.clone(),
        digest: Fingerprint::from_bytes(&bytes),
    };

    assert_eq!(
        recover_artifact(&bytes, &expectation, None),
        ArtifactRecovery::Loaded(stored)
    );
}
