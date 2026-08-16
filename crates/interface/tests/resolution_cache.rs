use interface::{
    DeclarationRef, Fingerprint, ResolutionCandidate, ResolutionIndex, ResolutionQuery,
};

fn query(scope: &str, name: &str) -> ResolutionQuery {
    ResolutionQuery {
        scope: scope.to_string(),
        namespace: "value".to_string(),
        name: name.to_string(),
        shape: "binary".to_string(),
    }
}

fn candidate(name: &str, body: &[u8]) -> ResolutionCandidate {
    ResolutionCandidate {
        reference: DeclarationRef::Subject {
            module_path: "api".to_string(),
            declaration_path: name.to_string(),
        },
        fingerprint: Fingerprint::from_bytes(body),
    }
}

#[test]
fn candidate_change_and_absent_bucket_addition_invalidate_dependencies() {
    let target = query("math", "add");
    let absent = query("math", "subtract");
    let baseline = ResolutionIndex::new([(target.clone(), candidate("add", b"one"))]);
    let target_dependency = baseline.dependency(target.clone());
    let absent_dependency = baseline.dependency(absent.clone());
    let changed = ResolutionIndex::new([
        (target.clone(), candidate("add", b"two")),
        (absent, candidate("subtract", b"one")),
    ]);

    assert!(!changed.dependency_is_current(&target_dependency));
    assert!(!changed.dependency_is_current(&absent_dependency));
}

#[test]
fn unrelated_scope_change_preserves_resolution_dependency() {
    let target = query("math", "add");
    let baseline = ResolutionIndex::new([(target.clone(), candidate("add", b"one"))]);
    let dependency = baseline.dependency(target.clone());
    let changed = ResolutionIndex::new([
        (target, candidate("add", b"one")),
        (query("text", "join"), candidate("join", b"one")),
    ]);

    assert!(changed.dependency_is_current(&dependency));
}

#[test]
fn resolution_map_and_candidate_insertion_order_are_nonsemantic() {
    let entries = vec![
        (query("math", "add"), candidate("second", b"two")),
        (query("math", "add"), candidate("first", b"one")),
        (query("text", "join"), candidate("join", b"three")),
    ];
    let forward = ResolutionIndex::new(entries.clone());
    let reverse = ResolutionIndex::new(entries.into_iter().rev());

    for query in [query("math", "add"), query("text", "join")] {
        assert_eq!(forward.dependency(query.clone()), reverse.dependency(query));
    }
}
