//! Package prepare uses the same [`ast::prepare_file_ast`] path as `ginc`.

use analysis::{CacheEngine, QueryEngine};
use crossbeam_channel::unbounded;
use std::path::PathBuf;

#[test]
fn package_typecheck_runs_prepare_before_transform() {
    let path = PathBuf::from("/tmp/prepare_parity_main.gin");
    let source = "main:\n    return 0\n";

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();
    engine.set_contents(&path, source.to_string());

    let outputs = engine.package_semantics(&[path]);
    assert_eq!(outputs.len(), 1);
    assert!(
        outputs[0].symptoms.is_empty(),
        "valid main should typecheck cleanly after prepare+transform"
    );
}
