use interface::{
    DeclarationRef, DependencyEdge, Fingerprint, PackageInstanceId, public_closure_fingerprint,
};

fn local(name: &str, body: &[u8]) -> DependencyEdge {
    DependencyEdge::ReachableLocal {
        reference: DeclarationRef::Subject {
            module_path: "implementation".to_string(),
            declaration_path: name.to_string(),
        },
        fingerprint: Fingerprint::from_bytes(body),
    }
}

fn external(body: &[u8]) -> DependencyEdge {
    DependencyEdge::External {
        reference: DeclarationRef::External {
            package: PackageInstanceId {
                package: "dep".to_string(),
                version: "1".to_string(),
                source: "registry".to_string(),
                instance: "root".to_string(),
            },
            module_path: "api".to_string(),
            declaration_path: "Value".to_string(),
        },
        fingerprint: Fingerprint::from_bytes(body),
    }
}

#[test]
fn folded_private_changes_do_not_invalidate_public_closure() {
    let left = [DependencyEdge::PrivateFolded {
        value: b"one".to_vec(),
    }];
    let right = [DependencyEdge::PrivateFolded {
        value: b"two".to_vec(),
    }];

    assert_eq!(
        public_closure_fingerprint(&left),
        public_closure_fingerprint(&right)
    );
}

#[test]
fn reachable_local_and_same_key_external_changes_invalidate_closure() {
    assert_ne!(
        public_closure_fingerprint(&[local("helper", b"one")]),
        public_closure_fingerprint(&[local("helper", b"two")])
    );
    assert_ne!(
        public_closure_fingerprint(&[external(b"one")]),
        public_closure_fingerprint(&[external(b"two")])
    );
}

#[test]
fn dependency_arena_order_does_not_change_closure_fingerprint() {
    let edges = vec![
        local("zeta", b"z"),
        external(b"external"),
        local("alpha", b"a"),
    ];

    assert_eq!(
        public_closure_fingerprint(&edges),
        public_closure_fingerprint(&edges.into_iter().rev().collect::<Vec<_>>())
    );
}
