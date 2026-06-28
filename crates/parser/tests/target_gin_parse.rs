//! Inline `target.gin`-like source: `target := Target.default` compile-time bind.

use ast::{BindValue, Expr};
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn parse_target_gin_fixture() {
    let src = r#"
Architecture is 'x86_64' or 'arm64' or 'wasm32'
Vendor is 'unknown'
OperatingSystem is 'unknown'
Default(value) has (default value)
Target has (arch Architecture, vendor Vendor, os OperatingSystem)
Target.Default(default: (arch: 'x86_64', vendor: 'unknown', os: 'unknown'))
target := Target.default
"#;
    let ast = src.parse_source_full().ast;
    let target = ast
        .defs
        .get(&Intern::from_ref("target"))
        .unwrap_or_else(|| {
            panic!(
                "expected `target` def, defs={:?}",
                ast.defs.keys().collect::<Vec<_>>()
            )
        });
    assert!(target.is_constant, "target should use `:=`");
    let mut ast = ast;
    use typecheck::comptime_classify::FileAstComptimeExt;
    ast.apply_comptime_classification();
    let target = ast.defs.get(&Intern::from_ref("target")).unwrap();
    assert!(
        target.is_compile_time,
        "target initializer should be comptime-classified"
    );
    let BindValue::Expr(expr) = &target.value else {
        panic!("expected Expr bind, got {:?}", target.value);
    };
    let Expr::FnCall(call) = &expr.value else {
        panic!("expected Target.default, got {:?}", expr.value);
    };
    assert_eq!(call.path.value.root.as_str(), "Target");
    assert_eq!(call.path.value.segments[0].as_str(), "default");
    assert!(call.args.is_none());
}
