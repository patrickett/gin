use interface::{InterfacePublication, PackageInstanceId, PublicInterface};

fn candidate(version: &str) -> PublicInterface {
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
fn successful_publication_is_authoritative() {
    let publication = InterfacePublication::resolve(candidate("2"), false, None);

    assert!(matches!(publication, InterfacePublication::Published(_)));
    assert!(!publication.is_stale());
    assert_eq!(publication.authoritative().unwrap().subject.version, "2");
}

#[test]
fn failed_publication_retains_only_an_explicitly_stale_success() {
    let unavailable = InterfacePublication::resolve(candidate("2"), true, None);
    let stale = InterfacePublication::resolve(candidate("2"), true, Some(candidate("1")));

    assert_eq!(unavailable, InterfacePublication::Unavailable);
    assert!(stale.is_stale());
    assert_eq!(stale.authoritative().unwrap().subject.version, "1");
}
