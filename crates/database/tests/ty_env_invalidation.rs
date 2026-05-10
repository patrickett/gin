use ast::prelude::Intern;
use crossbeam_channel::unbounded;
use database::SalsaQueryEngine;
use database::QueryEngine;
use std::path::PathBuf;

#[test]
fn package_ty_env_invalidates_on_file_change() {
    let (tx, _rx) = unbounded();
    let mut engine = SalsaQueryEngine::new(tx);

    let p1 = PathBuf::from("/tmp/a.gin");
    let p2 = PathBuf::from("/tmp/b.gin");

    let _ = engine.add_file(p1.clone());
    let _ = engine.add_file(p2.clone());

    engine.set_contents(&p1, "Bool is True or False\n".to_string());
    engine.set_contents(&p2, "Maybe[x] is Some(x) or None\n".to_string());

    let before = {
        let ty_env = engine.package_ty_env(&[p1.clone(), p2.clone()]);
        ty_env.tag_types.clone()
    };

    // Change the set of declared tags in b.gin.
    engine.set_contents(
        &p2,
        "Maybe[x] is Some(x) or None\nOther is X or Y\n".to_string(),
    );

    let after = {
        let ty_env = engine.package_ty_env(&[p1.clone(), p2.clone()]);
        ty_env.tag_types.clone()
    };
    assert_ne!(before, after);
    assert!(after.contains_key(&Intern::<String>::from_ref("Other")));
}
