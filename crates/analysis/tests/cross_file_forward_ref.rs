//! Package semantics: forward references across files must not be `UnknownSymbol`.

use analysis::{CacheEngine, QueryEngine};
use crossbeam_channel::unbounded;
use std::path::PathBuf;

fn unknown_foo_in_package(engine: &CacheEngine, paths: &[PathBuf]) -> bool {
    let outputs = engine.package_semantics(paths);
    outputs.iter().any(|out| {
        out.symptoms
            .iter()
            .any(|d| d.code.slug() == "type-unknown-symbol" && d.message.contains("foo"))
    })
}

#[test]
fn package_semantics_forward_ref_caller_first() {
    let caller_path = PathBuf::from("/tmp/gin_fwd_ref_caller_a.gin");
    let callee_path = PathBuf::from("/tmp/gin_fwd_ref_callee_a.gin");
    let caller_src = "bar() Int := foo() + 1\n";
    let callee_src = "foo() Int := 42\n";

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    let _ = engine.add_file(caller_path.clone());
    let _ = engine.add_file(callee_path.clone());
    engine.set_contents(&caller_path, caller_src.to_string());
    engine.set_contents(&callee_path, callee_src.to_string());

    assert!(
        !unknown_foo_in_package(&engine, &[caller_path, callee_path]),
        "caller-first: `foo` should resolve across files"
    );
}

#[test]
fn package_semantics_forward_ref_callee_first() {
    let caller_path = PathBuf::from("/tmp/gin_fwd_ref_caller_b.gin");
    let callee_path = PathBuf::from("/tmp/gin_fwd_ref_callee_b.gin");
    let caller_src = "bar() Int := foo() + 1\n";
    let callee_src = "foo() Int := 42\n";

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    let _ = engine.add_file(caller_path.clone());
    let _ = engine.add_file(callee_path.clone());
    engine.set_contents(&caller_path, caller_src.to_string());
    engine.set_contents(&callee_path, callee_src.to_string());

    assert!(
        !unknown_foo_in_package(&engine, &[callee_path, caller_path]),
        "callee-first: `foo` should resolve across files"
    );
}
