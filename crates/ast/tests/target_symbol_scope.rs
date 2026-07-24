//! `target Target` in gin_core/target/target.gin: `Target` must resolve (same-file declare).

use flask::CompileTarget;
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

const TARGET_FIXTURE: &str = r#"
Target has arch Architecture, vendor Vendor, os OperatingSystem
Target.Default has default: (arch: 'x86_64', vendor: 'unknown', os: 'unknown')

target Target
"#;

fn unknown_symbols(typed: &typecheck::TypedFileAst) -> Vec<String> {
    let mut names = Vec::new();
    for (_, f) in typed.declaration_flaws.iter() {
        if f.code.slug() == "type-unknown-symbol" {
            names.push(f.arg("name").unwrap_or("").to_string());
        }
    }
    for (_, f) in typed.all_flaws() {
        if f.code.slug() == "type-unknown-symbol" {
            names.push(f.arg("name").unwrap_or("").to_string());
        }
    }
    names
}

#[test]
fn target_gin_target_type_not_unknown_single_file() {
    let mut file_ast = TokenCursor::parse_source(TARGET_FIXTURE);
    let _ = typecheck::prepare_file_ast(&mut file_ast, &CompileTarget::Library);
    let typed = transform(&file_ast, FileId(0), &TransformCtx::new());
    let unknown = unknown_symbols(&typed);
    assert!(
        !unknown.iter().any(|n| n == "Target"),
        "Target should be in scope: unknown={unknown:?}, declaration_flaws={:?}",
        typed.declaration_flaws
    );
    assert!(
        !unknown.iter().any(|n| n == "Target.default"),
        "Target.default should resolve via Default trait: unknown={unknown:?}"
    );
}

#[test]
fn target_gin_target_type_not_unknown_minimal() {
    let source = r#"
Target has arch Architecture, vendor Vendor, os OperatingSystem
Target.Default has default: (arch: 'x86_64', vendor: 'unknown', os: 'unknown')

target Target
"#;
    let typed = transform_file_from_str(source, true);
    let unknown = unknown_symbols(&typed);
    assert!(
        !unknown.iter().any(|n| n == "Target"),
        "Target should be in scope: unknown={unknown:?}"
    );
}

fn transform_file_from_str(source: &str, prepare: bool) -> typecheck::TypedFileAst {
    let mut file_ast = TokenCursor::parse_source(source);
    if prepare {
        let _ = typecheck::prepare_file_ast(&mut file_ast, &CompileTarget::Library);
    }
    transform(&file_ast, FileId(0), &TransformCtx::new())
}
