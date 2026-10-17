use std::collections::BTreeMap;

use flask::{MinimumRequirement, Registry, RegistryError, Release, ReleaseStatus, SecurityPolicy};
use semver::Version;

fn requirements(entries: &[(&str, &str)]) -> BTreeMap<String, MinimumRequirement> {
    entries
        .iter()
        .map(|(alias, value)| {
            (
                alias.to_string(),
                MinimumRequirement::parse(alias, value).unwrap(),
            )
        })
        .collect()
}

fn add(registry: &mut Registry, name: &str, version: &str, dependencies: &[(&str, &str)]) {
    let version = Version::parse(version).unwrap();
    registry
        .insert(Release {
            name: name.into(),
            version: version.clone(),
            archive_sha256: "a".repeat(64),
            interface_sha256: "b".repeat(64),
            published_at: 0,
            dependencies: BTreeMap::new(),
            minimum_dependencies: Some(requirements(dependencies)),
        })
        .unwrap();
    registry
        .set_status(name, &version, ReleaseStatus::Eligible)
        .unwrap();
}

fn policy() -> SecurityPolicy {
    SecurityPolicy {
        now: 300_000,
        metadata_expires_at: 400_000,
        cooldown_seconds: 259_200,
    }
}

#[test]
fn diamond_selects_highest_minimum_and_ignores_new_releases() {
    let mut registry = Registry::default();
    add(&mut registry, "left", "1.0.0", &[("http", "1.0.0")]);
    add(&mut registry, "right", "1.0.0", &[("http", "1.2.0")]);
    add(&mut registry, "http", "1.0.0", &[]);
    add(&mut registry, "http", "1.2.0", &[]);
    let roots = requirements(&[("left", "1.0.0"), ("right", "1.0.0")]);
    let first = registry.resolve_minimum(&roots, policy()).unwrap();
    assert_eq!(
        first.packages["left@1.0.0"].dependencies["http"],
        "http@1.2.0"
    );
    assert!(!first.packages.contains_key("http@1.0.0"));
    add(&mut registry, "http", "1.9.0", &[]);
    assert_eq!(first, registry.resolve_minimum(&roots, policy()).unwrap());
}

#[test]
fn superseded_versions_still_contribute_requirements() {
    let mut registry = Registry::default();
    add(&mut registry, "left", "1.0.0", &[("http", "1.0.0")]);
    add(&mut registry, "http", "1.0.0", &[("socket", "1.4.0")]);
    add(&mut registry, "http", "1.2.0", &[]);
    add(&mut registry, "socket", "1.4.0", &[]);
    let selected = registry
        .resolve_minimum(
            &requirements(&[("left", "1.0.0"), ("http", "1.2.0")]),
            policy(),
        )
        .unwrap();
    assert!(selected.packages.contains_key("socket@1.4.0"));
    assert!(selected.packages["http@1.2.0"].dependencies.is_empty());
}

#[test]
fn requirement_cycles_terminate_and_major_versions_are_distinct() {
    let mut registry = Registry::default();
    add(&mut registry, "left", "1.0.0", &[("right", "1.0.0")]);
    add(&mut registry, "right", "1.0.0", &[("left", "1.0.0")]);
    add(&mut registry, "left", "2.0.0", &[]);
    let selected = registry
        .resolve_minimum(
            &requirements(&[("left", "1.0.0"), ("left2", "left@2.0.0")]),
            policy(),
        )
        .unwrap();
    assert_eq!(selected.packages.len(), 3);
    assert_eq!(selected.direct["left2"], "left@2.0.0");
    assert_eq!(
        selected.packages["right@1.0.0"].dependencies["left"],
        "left@1.0.0"
    );
}

#[test]
fn security_changes_fail_without_reselecting_and_superseded_blocks_do_not_change_selection() {
    let mut registry = Registry::default();
    add(&mut registry, "left", "1.0.0", &[("http", "1.0.0")]);
    add(&mut registry, "http", "1.0.0", &[]);
    add(&mut registry, "http", "1.2.0", &[]);
    add(&mut registry, "http", "1.9.0", &[]);
    let roots = requirements(&[("left", "1.0.0"), ("http", "1.2.0")]);
    registry
        .set_status(
            "http",
            &Version::new(1, 0, 0),
            ReleaseStatus::Blocked {
                reason: "old".into(),
            },
        )
        .unwrap();
    assert!(registry.resolve_minimum(&roots, policy()).is_ok());
    registry
        .set_status(
            "http",
            &Version::new(1, 2, 0),
            ReleaseStatus::Blocked {
                reason: "selected".into(),
            },
        )
        .unwrap();
    assert!(matches!(
        registry.resolve_minimum(&roots, policy()),
        Err(RegistryError::Blocked { .. })
    ));
    assert!(matches!(
        registry.resolve_minimum(
            &roots,
            SecurityPolicy {
                now: 400_000,
                ..policy()
            }
        ),
        Err(RegistryError::Expired)
    ));
}

#[test]
fn missing_minima_and_incompatible_legacy_ranges_fail() {
    let mut registry = Registry::default();
    add(&mut registry, "http", "1.2.0", &[]);
    assert!(matches!(
        registry.resolve_minimum(&requirements(&[("http", "1.0.0")]), policy()),
        Err(RegistryError::Missing(_))
    ));
    registry
        .insert(Release {
            name: "old".into(),
            version: Version::new(1, 0, 0),
            archive_sha256: "a".repeat(64),
            interface_sha256: "b".repeat(64),
            published_at: 0,
            dependencies: BTreeMap::from([(
                "http".into(),
                semver::VersionReq::parse("=1.2.0").unwrap(),
            )]),
            minimum_dependencies: None,
        })
        .unwrap();
    registry
        .set_status("old", &Version::new(1, 0, 0), ReleaseStatus::Eligible)
        .unwrap();
    assert!(
        registry
            .resolve_minimum(&requirements(&[("old", "1.0.0")]), policy())
            .unwrap_err()
            .to_string()
            .contains("republication")
    );
}

#[test]
fn explicit_updates_keep_other_selected_versions_and_record_disappearing_transitives() {
    let mut registry = Registry::default();
    add(&mut registry, "http", "1.0.0", &[("socket", "1.4.0")]);
    add(&mut registry, "http", "1.2.0", &[]);
    add(&mut registry, "http", "2.0.0", &[]);
    add(&mut registry, "socket", "1.4.0", &[]);
    add(&mut registry, "socket", "1.9.0", &[]);
    let roots = requirements(&[("http", "1.0.0")]);
    let target = MinimumRequirement::parse("http", "1.2.0").unwrap();
    let selected = registry
        .update_minimum(&roots, Some(&target), policy())
        .unwrap();
    assert!(selected.packages.contains_key("http@1.2.0"));
    assert!(selected.packages.contains_key("socket@1.4.0"));
    let config = flask::FlaskConfig::parse_manifest(
        "#settings\nmodule: 'app'\nversion: 0.1.0\n#dependencies\nhttp: 1.0.0\n",
    )
    .unwrap();
    let next = config.with_minimum_floors(&selected).unwrap();
    assert_eq!(
        registry
            .resolve_minimum(&next.registry_minimum_requirements().unwrap(), policy())
            .unwrap()
            .packages,
        selected.packages
    );
    let all = registry.update_minimum(&roots, None, policy()).unwrap();
    assert!(all.packages.contains_key("socket@1.9.0"));
    assert!(!all.packages.contains_key("http@2.0.0"));
    let missing = MinimumRequirement::parse("http", "2.0.0").unwrap();
    assert!(
        registry
            .update_minimum(&roots, Some(&missing), policy())
            .is_err()
    );
    let lower = MinimumRequirement::parse("socket", "1.0.0").unwrap();
    assert!(
        registry
            .update_minimum(&roots, Some(&lower), policy())
            .is_err()
    );
}

#[test]
fn large_iterative_graph_has_explicit_limits_and_does_not_use_the_call_stack() {
    let mut registry = Registry::default();
    for index in 0..4097 {
        let name = format!("package_{index}");
        let next = format!("package_{}", index + 1);
        let edges = if index < 4096 {
            vec![(next.as_str(), "1.0.0")]
        } else {
            vec![]
        };
        add(&mut registry, &name, "1.0.0", &edges);
    }
    assert!(matches!(
        registry.resolve_minimum(&requirements(&[("package_0", "1.0.0")]), policy()),
        Err(RegistryError::SearchLimit)
    ));
    assert_eq!(
        registry
            .resolve_minimum(&requirements(&[("package_1", "1.0.0")]), policy())
            .unwrap()
            .packages
            .len(),
        4096
    );
}
