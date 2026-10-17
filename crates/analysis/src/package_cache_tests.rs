use super::PackageCache;
use ast::ConstValue;
use crossbeam_channel::unbounded;
use internment::Intern;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn configured_package_analysis_uses_flask_target() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("gin-analysis-target-{suffix}"));
    std::fs::create_dir_all(&root).expect("test package directory");
    std::fs::write(
        root.join("mod.flask"),
        r#"#settings
module: "target-test"
version: 0.0.0
target: "wasm32-unknown-unknown"

#dependencies
"#,
    )
    .expect("Flask config");
    let source_path = root.join("main.gin");
    std::fs::write(&source_path, "bits := #index_bits(@())\n").expect("Gin source");

    let (tx, _rx) = unbounded();
    let cache = PackageCache::new(tx);
    cache.add_file(source_path.clone()).expect("cached file");
    let entry = cache
        .get_or_compute_package(std::slice::from_ref(&source_path))
        .expect("analyzed package");
    let typed = &entry.typed_asts[0];
    let bits = &typed.defs[&typecheck::DefId::named(Intern::from_ref("bits"))];
    let typecheck::BindBody::Expr(expression) = bits.body else {
        panic!("target query expression");
    };
    assert_eq!(
        typed.exprs.const_value_of(expression),
        Some(&Some(ConstValue::Int(32.into())))
    );
    std::fs::remove_dir_all(root).expect("remove test package");
}

#[test]
fn cached_analysis_cannot_hide_new_unverified_registry_dependencies() {
    let root = test_fixtures::TempPackage::new("registry_analysis_refresh");
    let manifest = root.path().join("mod.flask");
    let source = root.path().join("main.gin");
    std::fs::write(
        &manifest,
        r#"#settings
module: "app"
version: 1.0.0

#dependencies
"#,
    )
    .unwrap();
    std::fs::write(&source, "Choice is Left or Right\n").unwrap();
    let cache = PackageCache::for_test_with_file(source.clone());
    let old = cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    std::fs::write(
        &manifest,
        r#"#settings
module: "app"
version: 1.0.0

#dependencies
http: 1.0.0
"#,
    )
    .unwrap();
    let new = cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&old, &new));
    assert!(
        new.import_diags
            .values()
            .flatten()
            .any(|d| d.code.slug() == "package-registry-verification-failed")
    );
}

#[test]
fn minimum_analysis_cache_observes_invalid_and_changed_local_manifests() {
    let root = test_fixtures::TempPackage::new("minimum_analysis_refresh");
    root.write(
        "mod.flask",
        "#settings\nmodule: 'app'\nversion: 0.1.0\n#dependencies\ncore: './core'\n",
    );
    root.write(
        "core/mod.flask",
        "#settings\nmodule: 'core'\nversion: 0.1.0\n#dependencies\n",
    );
    root.write("main.gin", "Choice is Left or Right\n");
    let source = root.path().join("main.gin");
    let cache = PackageCache::for_test_with_file(source.clone());
    let old = cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    root.write("core/mod.flask", "invalid");
    let invalid = cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&old, &invalid));
    assert!(
        invalid
            .import_diags
            .values()
            .flatten()
            .any(|d| d.code.slug() == "package-invalid-workspace")
    );
    root.write(
        "core/mod.flask",
        "#settings\nmodule: 'core'\nversion: 0.1.0\n#dependencies\nhttp: 1.234567.0\n",
    );
    let changed = cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&invalid, &changed));
    assert!(
        changed
            .import_diags
            .values()
            .flatten()
            .any(|d| d.code.slug() == "package-registry-verification-failed")
    );
}
