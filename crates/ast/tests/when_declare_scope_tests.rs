//! Type-level `when` declarations must resolve names in file scope (imports + local defs).

use flask::CompileTarget;
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn transform_parsed(source: &str, prepare: bool) -> typecheck::TypedFileAst {
    let mut file_ast = TokenCursor::parse_source(source);
    if prepare {
        let _ = typecheck::prepare_file_ast(&mut file_ast, &CompileTarget::Library);
    }
    transform(&file_ast, FileId(0), &TransformCtx::new())
}

fn transform_prepared(source: &str) -> typecheck::TypedFileAst {
    transform_parsed(source, true)
}

#[test]
fn when_subject_unknown_without_prepare_is_declaration_flaw() {
    let source = r#"
Architecture is 'x86_64' or 'wasm32'

PointerSize is when target.arch is
    'x86_64' then BigInt
    'wasm32' then Int
    else BigInt
"#;
    let typed = transform_parsed(source, false);
    assert!(
        typed.declaration_flaws.iter().any(|(_, f)| {
            f.code.slug() == "type-unknown-symbol" && f.arg("name") == Some("target")
        }),
        "without prepare: expected UnknownSymbol for `target`, got: {:?}",
        typed.declaration_flaws
    );
}

#[test]
fn when_subject_unknown_bind_is_declaration_flaw() {
    let source = r#"
Architecture is 'x86_64' or 'wasm32'

PointerSize is when target.arch is
    'x86_64' then BigInt
    'wasm32' then Int
    else BigInt
"#;
    let typed = transform_prepared(source);
    assert!(
        typed.declaration_flaws.iter().any(|(_, f)| {
            f.code.slug() == "type-unknown-symbol" && f.arg("name") == Some("target")
        }),
        "expected UnknownSymbol for unimported `target`, got: {:?}",
        typed.declaration_flaws
    );
}

#[test]
fn when_subject_resolves_when_target_imported() {
    let source = r#"
Architecture is 'x86_64' or 'wasm32'
Vendor is 'unknown'
OperatingSystem is 'unknown'
Target has (arch Architecture, vendor Vendor, os OperatingSystem)
Target.Default(default: ( arch: 'x86_64', vendor: 'unknown', os: 'unknown', ))

target Target

PointerSize is when target.arch is
    'wasm32' then Int
    else BigInt
"#;
    let typed = transform_prepared(source);
    assert!(
        !typed
            .declaration_flaws
            .iter()
            .any(|(_, f)| f.code.slug() == "type-unknown-symbol" && f.arg("name") == Some("target")),
        "imported/local `target` should not be UnknownSymbol: {:?}",
        typed.declaration_flaws
    );
}

#[test]
fn when_subject_resolves_with_target_member_import() {
    let source = r#"
use '../target/'.target

Architecture is 'x86_64' or 'wasm32'

PointerSize is when target.arch is
    'x86_64' then BigInt
    'wasm32' then Int
    else BigInt
"#;
    let typed = transform_prepared(source);
    assert!(
        !typed
            .declaration_flaws
            .iter()
            .any(|(_, f)| f.code.slug() == "type-unknown-symbol" && f.arg("name") == Some("target")),
        "`use '../target/'.target` should bring `target` into scope: {:?}",
        typed.declaration_flaws
    );
}
