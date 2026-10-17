use flask::{Registry, RegistryError, Release, ReleaseStatus, SecurityPolicy};
use semver::{Version, VersionReq};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn requirements(entries: &[(&str, &str)]) -> BTreeMap<String, VersionReq> {
    entries
        .iter()
        .map(|(name, req)| (name.to_string(), VersionReq::parse(req).unwrap()))
        .collect()
}

fn release(name: &str, version: &str, deps: &[(&str, &str)]) -> Release {
    Release {
        name: name.into(),
        version: Version::parse(version).unwrap(),
        archive_sha256: format!("{:x}", Sha256::digest(b"source")),
        interface_sha256: format!("{:x}", Sha256::digest(b"interface")),
        published_at: 0,
        dependencies: requirements(deps),
        minimum_dependencies: None,
    }
}

fn policy() -> SecurityPolicy {
    SecurityPolicy {
        now: 300_000,
        metadata_expires_at: 400_000,
        cooldown_seconds: 259_200,
    }
}

fn eligible(registry: &mut Registry, release: Release) {
    let name = release.name.clone();
    let version = release.version.clone();
    registry.insert(release).unwrap();
    registry
        .set_status(&name, &version, ReleaseStatus::Eligible)
        .unwrap();
}

#[test]
fn immutable_releases_have_independent_revocable_status() {
    let mut registry = Registry::default();
    let pkg = release("http", "1.0.0", &[]);
    eligible(&mut registry, pkg.clone());
    assert!(matches!(
        registry.insert(pkg.clone()),
        Err(RegistryError::Immutable(_))
    ));
    registry
        .set_status(
            &pkg.name,
            &pkg.version,
            ReleaseStatus::Blocked {
                reason: "malware".into(),
            },
        )
        .unwrap();
    assert_eq!(registry.release("http", &pkg.version).unwrap(), &pkg);
    assert!(matches!(
        registry.check(&pkg, policy()),
        Err(RegistryError::Blocked { .. })
    ));
    pkg.verify_archive(b"source").unwrap();
    assert!(pkg.verify_archive(b"tampered").is_err());
}

#[test]
fn resolver_backtracks_and_shares_compatible_transitives() {
    let mut registry = Registry::default();
    for pkg in [
        release("app", "1.0.0", &[("http", "^1")]),
        release("http", "1.0.0", &[("socket", "^1")]),
        release("http", "1.1.0", &[("socket", "^2")]),
        release("socket", "1.0.0", &[]),
        release("socket", "2.0.0", &[]),
    ] {
        eligible(&mut registry, pkg);
    }
    let req = requirements(&[("app", "^1"), ("socket", "^1")]);
    let locked = registry.resolve(&req, policy()).unwrap();
    assert_eq!(locked.packages.len(), 3);
    assert!(locked.packages.contains_key("http@1.0.0"));
    registry
        .validate_resolution(&req, &locked, policy())
        .unwrap();
}

#[test]
fn cooldown_quarantine_and_expiration_apply_transitively() {
    let mut registry = Registry::default();
    eligible(&mut registry, release("http", "1.0.0", &[("socket", "^1")]));
    let mut socket = release("socket", "1.0.0", &[]);
    socket.published_at = policy().now - 10;
    registry.insert(socket.clone()).unwrap();
    let req = requirements(&[("http", "^1")]);
    assert!(registry.resolve(&req, policy()).is_err());
    registry
        .set_status("socket", &socket.version, ReleaseStatus::Eligible)
        .unwrap();
    assert!(registry.resolve(&req, policy()).is_err());
    let no_cooldown = SecurityPolicy {
        cooldown_seconds: 0,
        ..policy()
    };
    let locked = registry.resolve(&req, no_cooldown).unwrap();
    assert!(matches!(
        registry.validate_resolution(
            &req,
            &locked,
            SecurityPolicy {
                metadata_expires_at: policy().now,
                ..no_cooldown
            }
        ),
        Err(RegistryError::Expired)
    ));
    registry
        .set_status(
            "socket",
            &socket.version,
            ReleaseStatus::Blocked {
                reason: "campaign".into(),
            },
        )
        .unwrap();
    assert!(
        registry
            .validate_resolution(&req, &locked, no_cooldown)
            .is_err()
    );
}

#[test]
fn frozen_resolution_does_not_upgrade_when_a_release_appears() {
    let mut registry = Registry::default();
    eligible(&mut registry, release("http", "1.0.0", &[]));
    let req = requirements(&[("http", "^1")]);
    let locked = registry.resolve(&req, policy()).unwrap();
    eligible(&mut registry, release("http", "1.1.0", &[]));
    registry
        .validate_resolution(&req, &locked, policy())
        .unwrap();
    assert_eq!(locked.direct["http"], "http@1.0.0");
    assert_eq!(
        registry.resolve(&req, policy()).unwrap().direct["http"],
        "http@1.1.0"
    );
}

#[test]
fn missing_edges_cycles_unreachable_nodes_and_modified_digests_fail() {
    let mut registry = Registry::default();
    eligible(&mut registry, release("http", "1.0.0", &[("socket", "^1")]));
    eligible(&mut registry, release("socket", "1.0.0", &[]));
    let req = requirements(&[("http", "^1")]);
    let original = registry.resolve(&req, policy()).unwrap();
    let mut missing = original.clone();
    missing.packages.remove("socket@1.0.0");
    assert!(
        registry
            .validate_resolution(&req, &missing, policy())
            .is_err()
    );
    let mut cycle = original.clone();
    cycle
        .packages
        .get_mut("socket@1.0.0")
        .unwrap()
        .dependencies
        .insert("http".into(), "http@1.0.0".into());
    assert!(
        registry
            .validate_resolution(&req, &cycle, policy())
            .is_err()
    );
    let mut tampered = original.clone();
    tampered.packages.get_mut("http@1.0.0").unwrap().sha256 = "0".repeat(64);
    assert!(
        registry
            .validate_resolution(&req, &tampered, policy())
            .is_err()
    );
    let mut unused = original;
    unused.direct.clear();
    assert!(unused.validate(&BTreeMap::new()).is_err());
}

#[test]
fn release_records_round_trip_and_reject_unknown_fields() {
    let pkg = release("http", "1.0.0", &[]);
    let mut value = serde_json::to_value(&pkg).unwrap();
    assert_eq!(
        serde_json::from_value::<Release>(value.clone()).unwrap(),
        pkg
    );
    value["install-script"] = "malware".into();
    assert!(serde_json::from_value::<Release>(value).is_err());
}

#[test]
fn cycles_backtrack_to_an_acyclic_release_and_events_retain_history() {
    let mut registry = Registry::default();
    eligible(&mut registry, release("http", "1.0.0", &[]));
    eligible(&mut registry, release("http", "1.1.0", &[("http", "^1")]));
    let locked = registry
        .resolve(&requirements(&[("http", "^1")]), policy())
        .unwrap();
    assert_eq!(locked.direct["http"], "http@1.0.0");
    let events = registry.security_events();
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].revision, 1);
    assert_eq!(events[0].status, ReleaseStatus::Quarantined);
    assert_eq!(events[1].status, ReleaseStatus::Eligible);
}

#[test]
fn publication_verifies_the_archive_interface_and_version_claim() {
    use interface::{PackageInstanceId, PublicInterface};
    let mut registry = Registry::default();
    let mut old = PublicInterface::from_subject_and_declarations(
        PackageInstanceId {
            package: "http".into(),
            version: "1.0.0".into(),
            source: "flasks.io".into(),
            instance: "http".into(),
        },
        vec![],
    );
    let mut first = release("http", "1.0.0", &[]);
    first.interface_sha256 = format!("{:x}", Sha256::digest(old.canonical_bytes()));
    registry
        .validate_publication(&first, b"source", &old, None)
        .unwrap();
    registry.insert(first).unwrap();
    let mut new = old.clone();
    new.subject.version = "1.0.1".into();
    let mut patch = release("http", "1.0.1", &[]);
    patch.interface_sha256 = format!("{:x}", Sha256::digest(new.canonical_bytes()));
    registry
        .validate_publication(&patch, b"source", &new, Some(&old))
        .unwrap();
    assert!(
        registry
            .validate_publication(&patch, b"changed", &new, Some(&old))
            .is_err()
    );
    patch.version = Version::new(1, 1, 0);
    new.subject.version = "1.1.0".into();
    patch.interface_sha256 = format!("{:x}", Sha256::digest(new.canonical_bytes()));
    assert!(
        registry
            .validate_publication(&patch, b"source", &new, Some(&old))
            .is_err()
    );
    old.subject.package = "other".into();
    assert!(
        registry
            .validate_publication(&patch, b"source", &new, Some(&old))
            .is_err()
    );
}

#[test]
fn registry_names_and_versions_cannot_alias_case_insensitive_cache_paths() {
    for name in ["HTTP", "Http", "_http", "../http"] {
        assert!(
            Registry::default()
                .insert(release(name, "1.0.0", &[]))
                .is_err()
        );
    }
    for version in ["1.0.0-Alpha", "1.0.0+build"] {
        assert!(
            Registry::default()
                .insert(release("http", version, &[]))
                .is_err()
        );
    }
}
