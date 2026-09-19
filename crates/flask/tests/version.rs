use flask::{VersionBump, VersionError};
use interface::{
    DeclarationRef, Fingerprint, InterfaceDeclaration, PackageInstanceId, PublicInterface,
};
use semver::Version;

fn surface(entries: &[(&str, &str)]) -> PublicInterface {
    PublicInterface::from_subject_and_declarations(
        PackageInstanceId {
            package: "http".into(),
            version: "1.2.3".into(),
            source: "flasks.io".into(),
            instance: "http".into(),
        },
        entries
            .iter()
            .map(|(name, ty)| InterfaceDeclaration {
                reference: DeclarationRef::Subject {
                    module_path: "".into(),
                    declaration_path: name.to_string(),
                },
                fingerprint: Fingerprint::from_bytes(ty.as_bytes()),
            })
            .collect(),
    )
}

#[test]
fn bump_classifies_unchanged_additions_removals_and_changes() {
    let old = surface(&[("get", "request -> response")]);
    assert_eq!(
        VersionBump::compare(&old, &old).unwrap(),
        VersionBump::Patch
    );
    assert_eq!(
        VersionBump::compare(
            &old,
            &surface(&[
                ("get", "request -> response"),
                ("post", "request -> response")
            ])
        )
        .unwrap(),
        VersionBump::Minor
    );
    assert_eq!(
        VersionBump::compare(&old, &surface(&[])).unwrap(),
        VersionBump::Major
    );
    assert_eq!(
        VersionBump::compare(&old, &surface(&[("get", "request -> bytes")])).unwrap(),
        VersionBump::Major
    );
}

#[test]
fn version_claims_are_checked_with_overflow_and_prerelease_errors() {
    let old = Version::parse("1.2.3").unwrap();
    for (bump, expected) in [
        (VersionBump::Patch, "1.2.4"),
        (VersionBump::Minor, "1.3.0"),
        (VersionBump::Major, "2.0.0"),
    ] {
        let expected = Version::parse(expected).unwrap();
        assert_eq!(bump.next(&old).unwrap(), expected);
        bump.verify(&old, &expected).unwrap();
        assert!(bump.verify(&old, &old).is_err());
    }
    assert!(matches!(
        VersionBump::Major.next(&Version::new(u64::MAX, 0, 0)),
        Err(VersionError::Overflow)
    ));
    assert!(
        VersionBump::Patch
            .next(&Version::parse("1.0.0-beta.1").unwrap())
            .is_err()
    );
}

#[test]
fn unrelated_packages_and_unknown_closures_are_not_compatible() {
    let old = surface(&[]);
    let mut changed = old.clone();
    changed.subject.package = "unrelated".into();
    assert!(matches!(
        VersionBump::compare(&old, &changed),
        Err(VersionError::PackageMismatch)
    ));
    changed = old.clone();
    changed.public_closure_fingerprint = Fingerprint::from_bytes(b"new dependency");
    assert!(matches!(
        VersionBump::compare(&old, &changed),
        Err(VersionError::UnknownCompatibility)
    ));
}

#[test]
fn incomplete_schemas_and_changed_target_realizations_are_not_certified() {
    let old = surface(&[]);
    let mut changed = old.clone();
    changed.schema_version = 2;
    assert!(matches!(
        VersionBump::compare(&changed, &changed),
        Err(VersionError::SchemaMismatch)
    ));
    changed = old.clone();
    changed
        .target_realizations
        .push(interface::TargetRealization {
            target: "aarch64-apple-darwin".into(),
            semantic_surface_fingerprint: Fingerprint::from_bytes(b"surface"),
            public_closure_fingerprint: Fingerprint::from_bytes(b"closure"),
        });
    assert!(matches!(
        VersionBump::compare(&old, &changed),
        Err(VersionError::UnknownCompatibility)
    ));
}

#[test]
fn duplicate_declaration_identities_cannot_hide_a_changed_candidate() {
    use interface::{
        DeclarationRef, Fingerprint, InterfaceDeclaration, PackageInstanceId, PublicInterface,
    };
    let declaration = InterfaceDeclaration {
        reference: DeclarationRef::Subject {
            module_path: "".into(),
            declaration_path: "choose".into(),
        },
        fingerprint: Fingerprint::from_bytes(b"first"),
    };
    let old = PublicInterface::from_subject_and_declarations(
        PackageInstanceId {
            package: "pkg".into(),
            version: "1.0.0".into(),
            source: "test".into(),
            instance: "pkg".into(),
        },
        vec![declaration.clone()],
    );
    let mut new = old.clone();
    let mut changed = declaration;
    changed.fingerprint = Fingerprint::from_bytes(b"changed");
    new.declarations.insert(0, changed);
    assert!(flask::VersionBump::compare(&old, &new).is_err());
}

#[test]
fn resolution_candidate_changes_require_a_major_version() {
    let old = surface(&[("existing", "signature")]);
    let mut new = old.clone();
    new.resolution_fingerprint = interface::Fingerprint::from_bytes(b"new operator candidate");
    assert_eq!(
        VersionBump::compare(&old, &new).unwrap(),
        VersionBump::Major
    );
    assert_eq!(
        interface::PublicInterface::decode(&new.canonical_bytes()).unwrap(),
        new
    );
}
