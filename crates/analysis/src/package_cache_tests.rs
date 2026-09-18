use super::PackageCache;
use ast::{BindValue, ConstValue};
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
        root.join("flask.jsonc"),
        r#"{"name":"target-test","version":"0","authors":[],"target":"wasm32-unknown-unknown"}"#,
    )
    .expect("Flask config");
    let source_path = root.join("main.gin");
    std::fs::write(
        &source_path,
        r#"
Architecture is 'x86_64' or 'arm64' or 'wasm32'
Vendor is 'unknown'
OperatingSystem is 'unknown'
Default(value) has default value
Target has Default
    arch Architecture
    vendor Vendor
    os OperatingSystem
    Default.default: (arch: 'x86_64', vendor: 'unknown', os: 'unknown')
target Target
"#,
    )
    .expect("Gin source");

    let (tx, _rx) = unbounded();
    let cache = PackageCache::new(tx);
    cache.add_file(source_path.clone()).expect("cached file");
    let entry = cache
        .get_or_compute_package(std::slice::from_ref(&source_path))
        .expect("analyzed package");
    let target = entry.typed_asts[0]
        .eval_ast
        .defs
        .get(&Intern::from_ref("target"))
        .expect("target bind");
    let BindValue::Expr(expression) = &target.value else {
        panic!("target expression");
    };
    let Some(ConstValue::Record { fields }) = &expression.const_value else {
        panic!("target record");
    };
    assert_eq!(
        fields
            .iter()
            .find(|(name, _)| name.as_str() == "arch")
            .map(|(_, value)| value),
        Some(&ConstValue::Text("wasm32".into()))
    );
}

#[test]
fn cached_analysis_cannot_hide_new_unverified_registry_dependencies() {
    let root = test_fixtures::TempPackage::new("registry_analysis_refresh");
    let manifest = root.path().join("flask.jsonc");
    let source = root.path().join("main.gin");
    std::fs::write(
        &manifest,
        r#"{"name":"app","version":"1.0.0","authors":[]}"#,
    )
    .unwrap();
    std::fs::write(&source, "Choice is Left or Right\n").unwrap();
    let cache = PackageCache::for_test();
    cache.add_file(source.clone()).unwrap();
    let old = cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    std::fs::write(
        &manifest,
        r#"{"name":"app","version":"1.0.0","authors":[],"dependencies":{"http":{"version":"^1"}}}"#,
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
