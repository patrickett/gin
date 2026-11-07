//! Revision equivalence covers public derived-inspection caching, not semantic
//! typecheck incrementality: Session currently exposes no persistent semantic cache
//! or recheck counts. Each accepted revision is compared with a cold Session built
//! from its complete source.
use std::fs;
use std::path::PathBuf;

use ginterpreter::session::Session;
use serde_json::Value;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/revision_equivalence")
        .join(name);
    fs::read_to_string(path).unwrap()
}

fn without_volatile_metadata(value: &mut Value) {
    let object = value.as_object_mut().expect("inspection is an object");
    assert!(object.contains_key("cached"));
    assert!(object.contains_key("revision"));
    object.remove("cached");
    object.remove("revision");
}

fn equivalent(mut actual: Value, mut expected: Value) {
    without_volatile_metadata(&mut actual);
    without_volatile_metadata(&mut expected);
    assert_eq!(actual, expected);
}

fn cold(source: &str) -> Session {
    let mut session = Session::default();
    let declared = session.declare(source).expect("complete source checks");
    assert!(declared.accepted);
    session
}

#[test]
fn revisions_invalidate_changed_dependencies_and_match_cold_inspection() {
    let a = fixture("A.gin");
    let b = fixture("B.gin");
    let mut session = Session::default();
    let first = session.declare(&a).unwrap();
    assert!(first.accepted);

    for name in ["base", "top", "alone"] {
        assert_eq!(session.inspect(name)["cached"], false);
    }
    for name in ["base", "top", "alone"] {
        assert_eq!(session.inspect(name)["cached"], true);
    }

    let revision_source = format!("{a}\n{b}");
    let revision = session.declare(&b).unwrap();
    assert!(revision.accepted);
    assert!(revision.changed.contains(&"fn base/0".to_owned()));
    assert!(revision.changed.contains(&"fn top/0".to_owned()));
    assert!(revision.kept.contains(&"fn alone/0".to_owned()));

    let base = session.inspect("base");
    let top = session.inspect("top");
    let alone = session.inspect("alone");
    assert_eq!(base["cached"], false);
    assert_eq!(top["cached"], false);
    assert_eq!(alone["cached"], true);
    assert_eq!(top["value"]["value"]["value"], 8);

    let mut fresh = cold(&revision_source);
    equivalent(base, fresh.inspect("base"));
    equivalent(top, fresh.inspect("top"));
    equivalent(alone, fresh.inspect("alone"));
}

#[test]
fn rejected_revision_diagnostics_match_cold_check_and_preserve_cache() {
    let a = fixture("A.gin");
    let invalid = fixture("Invalid.gin");
    let mut session = cold(&a);
    assert_eq!(session.inspect("top")["cached"], false);
    assert_eq!(session.inspect("top")["cached"], true);

    let rejected = session.declare(&invalid).unwrap();
    assert!(!rejected.accepted);
    assert!(!rejected.flaws.is_empty());
    let attempted = format!("{a}\n{invalid}");
    let mut fresh = Session::default();
    let cold_rejected = fresh.declare(&attempted).unwrap();
    assert!(!cold_rejected.accepted);
    assert_eq!(
        rejected
            .flaws
            .iter()
            .map(|flaw| flaw.json())
            .collect::<Vec<_>>(),
        cold_rejected
            .flaws
            .iter()
            .map(|flaw| flaw.json())
            .collect::<Vec<_>>()
    );

    let retained = session.inspect("top");
    assert_eq!(retained["cached"], true);
    assert_eq!(retained["value"]["value"]["value"], 5);
    let mut fresh = cold(&a);
    equivalent(retained, fresh.inspect("top"));
}
