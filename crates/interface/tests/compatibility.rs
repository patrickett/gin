use interface::{
    Compatibility, DeclarationRef, Fingerprint, InterfaceDeclaration, PackageInstanceId,
    PublicInterface, classify_interfaces,
};

fn interface(declarations: &[(&str, &[u8])]) -> PublicInterface {
    PublicInterface::from_subject_and_declarations(
        PackageInstanceId {
            package: "pkg".to_string(),
            version: "1".to_string(),
            source: "workspace".to_string(),
            instance: "root".to_string(),
        },
        declarations
            .iter()
            .map(|(name, body)| InterfaceDeclaration {
                reference: DeclarationRef::Subject {
                    module_path: "api".to_string(),
                    declaration_path: (*name).to_string(),
                },
                fingerprint: Fingerprint::from_bytes(body),
            })
            .collect(),
    )
}

#[test]
fn declaration_addition_is_compatible_but_removal_or_change_breaks() {
    let base = interface(&[("value", b"v1")]);
    let added = interface(&[("value", b"v1"), ("other", b"v1")]);
    let changed = interface(&[("value", b"v2")]);

    assert_eq!(classify_interfaces(&base, &base), Compatibility::Unchanged);
    assert_eq!(
        classify_interfaces(&base, &added),
        Compatibility::Compatible
    );
    assert_eq!(classify_interfaces(&added, &base), Compatibility::Breaking);
    assert_eq!(
        classify_interfaces(&base, &changed),
        Compatibility::Breaking
    );
}
