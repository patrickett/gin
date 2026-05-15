use crossbeam_channel::unbounded;
use database::QueryEngine;
use database::SalsaQueryEngine;
use std::path::PathBuf;

#[test]
fn package_typecheck_symptoms_invalidate_on_file_change() {
    let (tx, _rx) = unbounded();
    let mut engine = SalsaQueryEngine::new(tx);

    let p1 = PathBuf::from("/tmp/a.gin");
    let p2 = PathBuf::from("/tmp/b.gin");

    let _ = engine.add_file(p1.clone());
    let _ = engine.add_file(p2.clone());

    // Set up two files: a.gin defines `foo()`, b.gin calls it.
    engine.set_contents(&p1, "foo Int: 42\n".to_string());
    engine.set_contents(&p2, "main:\n    x: foo()\nreturn x\n".to_string());

    let before = engine.typecheck_package(&[p1.clone(), p2.clone()]);

    // Remove `foo` from b.gin's visible scope by pointing to a file without it.
    engine.set_contents(&p1, "\n".to_string());

    let after = engine.typecheck_package(&[p1.clone(), p2.clone()]);
    assert_ne!(before, after);
}
