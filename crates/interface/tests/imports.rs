use interface::{
    DeclarationRef, Fingerprint, IndexedDeclaration, IndexedInterface, IndexedInterfaceError,
};

fn reference(name: &str) -> DeclarationRef {
    DeclarationRef::Subject {
        module_path: "api".to_string(),
        declaration_path: name.to_string(),
    }
}

fn declaration(name: &str, dependencies: &[&str]) -> IndexedDeclaration {
    let body = name.as_bytes().to_vec();
    IndexedDeclaration {
        reference: reference(name),
        fingerprint: Fingerprint::from_bytes(&body),
        dependencies: dependencies.iter().map(|name| reference(name)).collect(),
        body,
    }
}

#[test]
fn random_access_decodes_only_the_visited_transitive_closure() {
    let interface = IndexedInterface::new([
        declaration("root", &["left", "right"]),
        declaration("left", &["shared"]),
        declaration("right", &["shared"]),
        declaration("shared", &["root"]),
        declaration("unrelated", &[]),
    ])
    .unwrap();
    let closure = interface.decode_closure(&reference("root")).unwrap();
    let names: Vec<_> = closure
        .iter()
        .map(|declaration| &declaration.reference)
        .collect();

    assert_eq!(closure.len(), 4);
    assert!(!names.contains(&&reference("unrelated")));
}

#[test]
fn random_access_verifies_each_decoded_shallow_body() {
    let mut corrupt = declaration("corrupt", &[]);
    corrupt.fingerprint = Fingerprint::from_bytes(b"different");
    let interface = IndexedInterface::new([corrupt]).unwrap();

    assert_eq!(
        interface.decode_closure(&reference("corrupt")),
        Err(IndexedInterfaceError::FingerprintMismatch(Box::new(
            reference("corrupt")
        )))
    );
}
