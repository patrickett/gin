//! Compile target: Default trait materialization and flask triple merge.

use ast::ConstValue;
use flask::{CompileTarget, TargetTriple};
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::prepare_parse_ast;

fn prepare_source(source: &str, entry: &CompileTarget) -> ast::FileAst {
    let mut ast = TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut ast, entry);
    ast
}

#[test]
fn type_static_default_field_expands_target_default() {
    let source = r#"
Architecture is 'x86_64' or 'arm64'
Vendor is 'unknown'
OperatingSystem is 'unknown'
Default(value) has default value
Target has Default
    arch Architecture
    vendor Vendor
    os OperatingSystem
    Default.default: ( arch: 'x86_64', vendor: 'unknown', os: 'unknown', )
target := Target.default
"#;
    let ast = prepare_source(source, &CompileTarget::Library);
    let target = ast.defs.get(&Intern::from_ref("target")).expect("target");
    let cv = match &target.value {
        ast::BindValue::Expr(e) => e.const_value.as_ref().expect("folded target"),
        _ => panic!("expected expr bind"),
    };
    let arch = match cv {
        ConstValue::Record { fields } => fields
            .iter()
            .find(|(n, _)| n.as_str() == "arch")
            .map(|(_, v)| v),
        other => panic!("expected Target record, got {other:?}"),
    }
    .expect("arch");
    assert_eq!(arch.as_str(), Some("x86_64"));
}

#[test]
fn default_trait_materializes_kind_variant() {
    let source = r#"
Default(value) has default value

Kind is A or B or C
Kind has Default
    Default.default: Kind.A

k Kind
"#;
    let ast = prepare_source(source, &CompileTarget::Library);
    let k = ast.defs.get(&Intern::from_ref("k")).expect("k bind");
    let cv = match &k.value {
        ast::BindValue::Expr(e) => e.const_value.as_ref().expect("const k"),
        _ => panic!("expected expr bind"),
    };
    match cv {
        ConstValue::Tag { name, .. } => assert_eq!(name.as_str(), "A"),
        other => panic!("expected Kind.A, got {other:?}"),
    }
}

#[test]
fn record_default_does_not_leak_top_level_defs() {
    let source = r#"
Target has Default
    arch Architecture
    Default.default: ( arch: 'x86_64', )
target Target
"#;
    let ast = TokenCursor::parse_source(source);
    assert!(ast.defs.contains_key(&Intern::from_ref("target")));
    assert!(!ast.defs.contains_key(&Intern::from_ref("arch")));
}

#[test]
fn full_target_fixture_parses() {
    let source = r#"
Architecture is 'x86_64' or 'arm64' or 'wasm32'
Vendor is 'unknown'
OperatingSystem is 'linux' or 'macOS' or 'windows' or 'unknown'
Default(value) has default value
Target has Default
    arch Architecture
    vendor Vendor
    os OperatingSystem
    Default.default: ( arch: 'x86_64', vendor: 'unknown', os: 'unknown', )
target Target
"#;
    let ast = TokenCursor::parse_source(source);
    assert!(!ast.tags.is_empty(), "tags empty");
    let keys: Vec<_> = ast.defs.keys().map(|k| k.as_str()).collect();
    assert!(
        ast.defs.contains_key(&Intern::from_ref("target")),
        "defs={keys:?}"
    );
}

#[test]
fn target_merge_overrides_default_arch() {
    let source = r#"
Architecture is 'x86_64' or 'arm64' or 'wasm32'
Vendor is 'unknown'
OperatingSystem is 'linux' or 'macOS' or 'windows' or 'unknown'

Default(value) has default value

Target has Default
    arch Architecture
    vendor Vendor
    os OperatingSystem
    Default.default: ( arch: 'x86_64', vendor: 'unknown', os: 'unknown', )

target Target
"#;
    let triple = TargetTriple::parse("wasm32-unknown-unknown").unwrap();
    let entry = CompileTarget::Concrete(triple);
    let ast = prepare_source(source, &entry);
    assert!(
        ast.defs.contains_key(&Intern::from_ref("target")),
        "prepare defs: {:?}",
        ast.defs.keys().map(|k| k.as_str()).collect::<Vec<_>>()
    );
    let target = ast.defs.get(&Intern::from_ref("target")).expect("target");
    let cv = match &target.value {
        ast::BindValue::Expr(e) => e.const_value.as_ref().expect("target const"),
        _ => panic!("expected materialized target"),
    };
    let arch = match cv {
        ConstValue::Record { fields } => fields
            .iter()
            .find(|(n, _)| n.as_str() == "arch")
            .map(|(_, v)| v),
        _ => panic!("expected record"),
    }
    .expect("arch field");
    assert_eq!(arch.as_str(), Some("wasm32"));
}
