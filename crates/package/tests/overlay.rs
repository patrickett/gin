//! An overlay stands in for files on disk (an editor's unsaved buffers).
use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use package::{GraphLimits, Package, PackageError};
use typecheck::Limits;

const MANIFEST: &str = "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n";

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("mod.flask"), MANIFEST).unwrap();
    fs::write(root.path().join("main.gin"), "use app.util five\nagain() in 0 to 16: return five()\n").unwrap();
    fs::create_dir(root.path().join("util")).unwrap();
    fs::write(root.path().join("util/five.gin"), "five() in 0 to 16: return 5\n").unwrap();
    root
}

#[test]
fn the_overlay_text_is_checked_instead_of_the_file_on_disk() {
    let dir = root();
    let util = dir.path().canonicalize().unwrap().join("util/five.gin");
    let open = || Package::open(&dir.path().canonicalize().unwrap()).unwrap();
    // Without an overlay the package checks.
    assert!(open().check(GraphLimits::default(), Limits::default()).is_ok());
    // With one that makes `five` too narrow, the same package fails in that module.
    let overlay = Arc::new(BTreeMap::from([(util.clone(), "five() in 0 to 3: return 5\n".to_owned())]));
    let error = open().with_overlay(overlay).check(GraphLimits::default(), Limits::default()).unwrap_err();
    assert!(matches!(&error, PackageError::Check { module, .. } if module == "app.util"), "{error:?}");
    // The disk is untouched.
    assert_eq!(fs::read_to_string(util).unwrap(), "five() in 0 to 16: return 5\n");
}

#[test]
fn an_overlay_with_a_syntax_error_is_a_load_error_for_that_file() {
    let dir = root();
    let main = dir.path().canonicalize().unwrap().join("main.gin");
    let overlay = Arc::new(BTreeMap::from([(main, "again(: return\n".to_owned())]));
    let error = Package::open(&dir.path().canonicalize().unwrap()).unwrap().with_overlay(overlay).check(GraphLimits::default(), Limits::default()).unwrap_err();
    assert!(matches!(error, PackageError::Graph(_)), "{error:?}");
}
