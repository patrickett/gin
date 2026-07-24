//! Compile-time `:=` binds and record construction must not trigger runtime linearity flaws.

use flask::CompileTarget;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn lin_not_consumed_names(typed: &typecheck::TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .filter(|&(_, flaw)| flaw.code.slug() == "type-lin-value-not-consumed")
        .map(|(_, flaw)| flaw.arg("name").unwrap_or("").to_string())
        .collect()
}

fn transform_prepared(source: &str) -> typecheck::TypedFileAst {
    let mut file_ast = parser::cursor::TokenCursor::parse_source(source);
    let _ = typecheck::prepare_file_ast(&mut file_ast, &CompileTarget::Library);
    transform(&file_ast, FileId(0), &TransformCtx::new())
}

const TARGET_FIXTURE: &str = r#"
Architecture is 'x86_64' or 'arm64' or 'wasm32'
Vendor is 'unknown'
OperatingSystem is 'unknown'
Default(value) has default value
Target has arch Architecture, vendor Vendor, os OperatingSystem
Target.Default has default: ( arch: 'x86_64', vendor: 'unknown', os: 'unknown', )
target := Target.default
"#;

#[test]
fn target_gin_no_lin_on_record_fields() {
    let typed = transform_prepared(TARGET_FIXTURE);
    let lin = lin_not_consumed_names(&typed);
    for name in ["arch", "vendor", "os", "target"] {
        assert!(
            !lin.iter().any(|n| n == name),
            "unexpected LinValueNotConsumed for `{name}`: {lin:?}"
        );
    }
}

#[test]
fn compile_time_record_default_no_phantom_locals() {
    let typed = transform_prepared(TARGET_FIXTURE);
    let lin = lin_not_consumed_names(&typed);
    for name in ["arch", "vendor", "os", "target"] {
        assert!(
            !lin.iter().any(|n| n == name),
            "unexpected LinValueNotConsumed for `{name}`: {lin:?}"
        );
    }
}

#[test]
fn runtime_fn_read_target_no_lin_on_target() {
    let source = format!(
        "{TARGET_FIXTURE}
read_target() Target: target
return

main:
    read_target()
return 0
"
    );
    let typed = transform_prepared(&source);
    let lin = lin_not_consumed_names(&typed);
    assert!(
        !lin.iter().any(|n| n == "target"),
        "module constant `target` should not be linearly checked in runtime fn: {lin:?}"
    );
}
