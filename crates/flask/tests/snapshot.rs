use ed25519_dalek::SigningKey;
use flask::{MetadataError, Registry, Release, ReleaseStatus, SignedSnapshot, Snapshot, TrustRoot};
use semver::Version;
use std::collections::BTreeMap;

fn keys() -> BTreeMap<String, SigningKey> {
    BTreeMap::from([
        ("primary".into(), SigningKey::from_bytes(&[7; 32])),
        ("secondary".into(), SigningKey::from_bytes(&[9; 32])),
    ])
}
fn trust() -> TrustRoot {
    TrustRoot {
        registry: "flasks.io".into(),
        threshold: 2,
        keys: keys()
            .iter()
            .map(|(id, key)| {
                (
                    id.clone(),
                    key.verifying_key()
                        .to_bytes()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                )
            })
            .collect(),
    }
}
fn registry() -> Registry {
    let mut registry = Registry::default();
    registry
        .insert(Release {
            name: "http".into(),
            version: Version::new(1, 0, 0),
            archive_sha256: "a".repeat(64),
            interface_sha256: "b".repeat(64),
            published_at: 0,
            dependencies: BTreeMap::new(),
            minimum_dependencies: None,
        })
        .unwrap();
    registry
        .set_status("http", &Version::new(1, 0, 0), ReleaseStatus::Eligible)
        .unwrap();
    registry
}
fn snapshot() -> Snapshot {
    registry().snapshot("flasks.io".into(), 1, 300_000, 400_000)
}
fn sign(snapshot: &Snapshot) -> SignedSnapshot {
    SignedSnapshot::sign(snapshot, &keys()).unwrap()
}

#[test]
fn threshold_authenticates_exact_payload_and_registry() {
    let signed = sign(&snapshot());
    let verified = signed.verify(&trust(), 310_000, None).unwrap();
    assert_eq!(verified.registry().security_events().len(), 2);
    let mut tampered = signed.clone();
    tampered.payload.push(' ');
    assert!(matches!(
        tampered.verify(&trust(), 310_000, None),
        Err(MetadataError::Signature)
    ));
    let mut insufficient = signed.clone();
    insufficient.signatures[1] = insufficient.signatures[0].clone();
    assert!(matches!(
        insufficient.verify(&trust(), 310_000, None),
        Err(MetadataError::Signature)
    ));
    let mut wrong = trust();
    wrong.registry = "attacker.example".into();
    assert!(signed.verify(&wrong, 310_000, None).is_err());
}

#[test]
fn aliases_for_one_key_cannot_satisfy_two_signer_threshold() {
    let mut keys = keys();
    keys.insert("secondary".into(), keys["primary"].clone());
    let signed = SignedSnapshot::sign(&snapshot(), &keys).unwrap();
    let mut trust = trust();
    trust
        .keys
        .insert("secondary".into(), trust.keys["primary"].clone());
    assert!(matches!(
        signed.verify(&trust, 310_000, None),
        Err(MetadataError::Signature)
    ));
}

#[test]
fn freshness_is_required_even_offline_but_expired_history_can_be_authenticated() {
    let signed = sign(&snapshot());
    assert!(matches!(
        signed.verify(&trust(), 299_999, None),
        Err(MetadataError::Time)
    ));
    assert!(matches!(
        signed.verify(&trust(), 400_000, None),
        Err(MetadataError::Time)
    ));
    let old = signed.authenticate(&trust()).unwrap();
    assert!(old.policy(400_000, 0).is_err());
    let mut next = snapshot();
    next.revision = 2;
    next.issued_at = 400_000;
    next.expires_at = 500_000;
    sign(&next).verify(&trust(), 410_000, Some(&old)).unwrap();
}

#[test]
fn signed_rollback_equivocation_and_rewritten_history_are_rejected() {
    let old = sign(&snapshot()).verify(&trust(), 310_000, None).unwrap();
    let mut next = snapshot();
    next.expires_at += 1;
    assert!(matches!(
        sign(&next).verify(&trust(), 310_000, Some(&old)),
        Err(MetadataError::Rollback)
    ));
    next.revision = 2;
    let newer = sign(&next).verify(&trust(), 310_000, Some(&old)).unwrap();
    assert!(matches!(
        sign(&snapshot()).verify(&trust(), 310_000, Some(&newer)),
        Err(MetadataError::Rollback)
    ));
    next.releases[0].archive_sha256 = "c".repeat(64);
    assert!(matches!(
        sign(&next).verify(&trust(), 310_000, Some(&old)),
        Err(MetadataError::History)
    ));
    next = snapshot();
    next.revision = 2;
    next.events[1].status = ReleaseStatus::Blocked {
        reason: "rewritten".into(),
    };
    assert!(matches!(
        sign(&next).verify(&trust(), 310_000, Some(&old)),
        Err(MetadataError::History)
    ));
    next.releases.clear();
    next.events.clear();
    assert!(matches!(
        sign(&next).verify(&trust(), 310_000, Some(&old)),
        Err(MetadataError::History)
    ));
}

#[test]
fn appended_security_block_preserves_immutable_release() {
    let old = sign(&snapshot()).verify(&trust(), 310_000, None).unwrap();
    let mut registry = registry();
    registry
        .set_status(
            "http",
            &Version::new(1, 0, 0),
            ReleaseStatus::Blocked {
                reason: "campaign".into(),
            },
        )
        .unwrap();
    let next = sign(&registry.snapshot("flasks.io".into(), 2, 310_000, 410_000))
        .verify(&trust(), 310_000, Some(&old))
        .unwrap();
    let release = next
        .registry()
        .release("http", &Version::new(1, 0, 0))
        .unwrap();
    assert_eq!(release, &old.snapshot().releases[0]);
    assert!(
        next.registry()
            .check(release, next.policy(310_000, 0).unwrap())
            .is_err()
    );
}

#[test]
fn schema_two_appends_minimum_records_without_rewriting_schema_one_history() {
    let old = sign(&snapshot()).verify(&trust(), 310_000, None).unwrap();
    let mut registry = registry();
    registry
        .insert(Release {
            name: "json".into(),
            version: Version::new(1, 0, 0),
            archive_sha256: "c".repeat(64),
            interface_sha256: "d".repeat(64),
            published_at: 0,
            dependencies: BTreeMap::new(),
            minimum_dependencies: Some(BTreeMap::from([(
                "http".into(),
                flask::MinimumRequirement::parse("http", "1.0.0").unwrap(),
            )])),
        })
        .unwrap();
    registry
        .set_status("json", &Version::new(1, 0, 0), ReleaseStatus::Eligible)
        .unwrap();
    let snapshot = registry.snapshot("flasks.io".into(), 2, 310_000, 410_000);
    assert_eq!(snapshot.schema, 2);
    let next = sign(&snapshot)
        .verify(&trust(), 310_000, Some(&old))
        .unwrap();
    assert_eq!(
        next.registry()
            .release("http", &Version::new(1, 0, 0))
            .unwrap(),
        &old.snapshot().releases[0]
    );
    let mut wrong_schema = snapshot.clone();
    wrong_schema.schema = 1;
    assert!(
        sign(&wrong_schema)
            .verify(&trust(), 310_000, Some(&old))
            .is_err()
    );
    let mut rewritten = snapshot;
    rewritten.revision = 3;
    rewritten
        .releases
        .iter_mut()
        .find(|r| r.name == "json")
        .unwrap()
        .minimum_dependencies
        .as_mut()
        .unwrap()
        .clear();
    assert!(matches!(
        sign(&rewritten).verify(&trust(), 310_000, Some(&next)),
        Err(MetadataError::History)
    ));
}
