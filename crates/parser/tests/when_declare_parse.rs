//! Type-level `PointerSize is when target.arch is …` must parse as `DeclareValue::When`.

use ast::DeclareValue;
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn pointer_size_is_when_single_line_parses() {
    let source = "PointerSize is when target.arch is 'x86_64' then BigInt else Int";
    let ast = source.parse_source_full().ast;
    let decl = ast
        .tags
        .get(&Intern::new("PointerSize".to_string()))
        .unwrap();
    assert!(matches!(&decl.value, DeclareValue::When(_)));
}

#[test]
fn pointer_size_is_when_multiline_parses() {
    let source = r#"
PointerSize is when target.arch is
    'x86_64' then BigInt
    'wasm32' then Int
    else BigInt
"#;
    let ast = source.parse_source_full().ast;
    let decl = ast
        .tags
        .get(&Intern::new("PointerSize".to_string()))
        .expect("PointerSize tag");
    match &decl.value {
        DeclareValue::When(w) => {
            assert!(w.subject.is_some());
            assert!(!w.arms.is_empty());
        }
        other => panic!("expected DeclareValue::When, got {other:?}"),
    }
}

#[test]
fn pointer_size_is_when_or_literals_single_arm_parses() {
    let source = r#"
PointerSize is when target.arch is
    'x86_64' or 'arm64' then BigInt
    'wasm32' then Int
    else BigInt
"#;
    let ast = source.parse_source_full().ast;
    let decl = ast
        .tags
        .get(&Intern::new("PointerSize".to_string()))
        .expect("PointerSize tag");
    match &decl.value {
        DeclareValue::When(w) => {
            use ast::WhenArm;
            let is_arms: Vec<_> = w
                .arms
                .iter()
                .filter_map(|a| match a {
                    WhenArm::Is { pattern, .. } => Some(pattern.value.clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                is_arms.len(),
                3,
                "expected three is arms (x86_64, arm64, wasm32): {is_arms:?}"
            );
        }
        other => panic!("expected DeclareValue::When, got {other:?}"),
    }
}
