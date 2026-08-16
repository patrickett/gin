use interface::{
    DeclarationRef, Fingerprint, InterfaceDeclaration, PackageInstanceId, PublicInterface,
};

fn subject(version: &str, source: &str, instance: &str) -> PackageInstanceId {
    PackageInstanceId {
        package: "pkg".to_string(),
        version: version.to_string(),
        source: source.to_string(),
        instance: instance.to_string(),
    }
}

fn local_declaration() -> InterfaceDeclaration {
    InterfaceDeclaration {
        reference: DeclarationRef::Subject {
            module_path: "api".to_string(),
            declaration_path: "Value".to_string(),
        },
        fingerprint: Fingerprint::from_bytes(b"shallow body"),
    }
}

#[test]
fn subject_identity_changes_base_but_reuses_subject_relative_shallow_bytes() {
    let declaration = local_declaration();
    let baseline = PublicInterface::from_subject_and_declarations(
        subject("1", "registry:a", "root"),
        vec![declaration.clone()],
    );
    for changed in [
        subject("2", "registry:a", "root"),
        subject("1", "registry:b", "root"),
        subject("1", "registry:a", "other"),
    ] {
        let interface =
            PublicInterface::from_subject_and_declarations(changed, vec![declaration.clone()]);
        assert_ne!(baseline.base_fingerprint(), interface.base_fingerprint());
        assert_eq!(
            baseline.declarations[0].fingerprint,
            interface.declarations[0].fingerprint
        );
    }
}

#[test]
fn external_identity_components_change_public_closure_fingerprint() {
    let external = |version: &str, source: &str, instance: &str| InterfaceDeclaration {
        reference: DeclarationRef::External {
            package: subject(version, source, instance),
            module_path: "dep".to_string(),
            declaration_path: "Value".to_string(),
        },
        fingerprint: Fingerprint::from_bytes(b"external body"),
    };
    let baseline = PublicInterface::from_subject_and_declarations(
        subject("1", "workspace", "root"),
        vec![external("1", "registry:a", "root")],
    );
    for declaration in [
        external("2", "registry:a", "root"),
        external("1", "registry:b", "root"),
        external("1", "registry:a", "other"),
    ] {
        let changed = PublicInterface::from_subject_and_declarations(
            subject("1", "workspace", "root"),
            vec![declaration],
        );
        assert_ne!(
            baseline.public_closure_fingerprint,
            changed.public_closure_fingerprint
        );
    }
}
