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
        Some(&ConstValue::String("wasm32".into()))
    );
}
