use ast::prelude::Intern;
use crossbeam_channel::unbounded;
use database::{InputDatabase, set_file_contents};
use database::{PackageFiles, package_ty_env};
use std::path::PathBuf;

#[test]
fn package_ty_env_invalidates_on_file_change() {
    let (tx, _rx) = unbounded();
    let mut db = InputDatabase::new(tx);

    let f1 = database::File::new(
        &db,
        PathBuf::from("/tmp/a.gin"),
        "Bool is True or False\n".to_string(),
    );
    let f2 = database::File::new(
        &db,
        PathBuf::from("/tmp/b.gin"),
        "Maybe[x] is Some(x) or None\n".to_string(),
    );

    let before = {
        let pkg = PackageFiles::new(&db, vec![f1, f2]);
        package_ty_env(&db, pkg).tag_types.clone()
    };

    // Change the set of declared tags in b.gin.
    set_file_contents(
        &mut db,
        f2,
        "Maybe[x] is Some(x) or None\nOther is X or Y\n".to_string(),
    );

    let after = {
        let pkg = PackageFiles::new(&db, vec![f1, f2]);
        package_ty_env(&db, pkg).tag_types.clone()
    };
    assert_ne!(before, after);
    assert!(after.contains_key(&Intern::<String>::from_ref("Other")));
}
