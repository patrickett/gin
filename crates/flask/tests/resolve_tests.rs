use flask::{FlaskPathExt, NestedPackageResolveError, NestedPackageTarget};
use std::fs;

fn tmp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "gin_flask_resolve_test_{}_{}",
        name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&p);
    p
}

#[test]
fn nested_path_two_segments_requires_flask_at_each_level() {
    let root = tmp("two_seg");
    fs::create_dir_all(&root).unwrap();
    let dep = root.join("dep");
    fs::create_dir_all(dep.join("seg1/seg2")).unwrap();
    fs::write(dep.join("flask.jsonc"), "{}").unwrap();
    fs::write(dep.join("seg1/flask.jsonc"), "{}").unwrap();
    fs::write(dep.join("seg1/seg2/flask.jsonc"), "{}").unwrap();

    let t = dep.resolve_nested_package_path(&["seg1", "seg2"]).unwrap();
    match t {
        NestedPackageTarget::FolderModule(p) => assert_eq!(p, dep.join("seg1/seg2")),
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn nested_path_missing_intermediate_flask_errors() {
    let root = tmp("missing_mid");
    fs::create_dir_all(&root).unwrap();
    let dep = root.join("dep");
    fs::create_dir_all(dep.join("seg1/seg2")).unwrap();
    fs::write(dep.join("flask.jsonc"), "{}").unwrap();
    fs::write(dep.join("seg1/seg2/flask.jsonc"), "{}").unwrap();

    let err = dep
        .resolve_nested_package_path(&["seg1", "seg2"])
        .unwrap_err();
    assert!(matches!(
        err,
        NestedPackageResolveError::IntermediateNotFolderModule { .. }
    ));
    let _ = fs::remove_dir_all(&root);
}
