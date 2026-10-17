//! Compile-time `:=` binds and record construction must not trigger runtime linearity flaws.

mod support;
use support::transform_prepared;

fn lin_not_consumed_names(typed: &typecheck::TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .filter(|&(_, flaw)| flaw.code.slug() == "type-lin-value-not-consumed")
        .map(|(_, flaw)| flaw.arg("name").unwrap_or("").to_string())
        .collect()
}

const TARGET_FIXTURE: &str = r#"
Architecture is 'x86_64' or 'aarch64' or 'wasm32'
Vendor is 'unknown'
OperatingSystem is 'unknown'
Default(value) has default value
Target has Default
    arch Architecture
    vendor Vendor
    os OperatingSystem
    Default.default: [ arch: 'x86_64', vendor: 'unknown', os: 'unknown', ]
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
fn runtime_fn_read_target_no_lin_on_target() {
    let source = format!(
        "{TARGET_FIXTURE}\nread_target() Target: return target\n    return\n\nmain:\n    read_target()\n    return 0\n"
    );
    let typed = transform_prepared(&source);
    let lin = lin_not_consumed_names(&typed);
    assert!(
        !lin.iter().any(|n| n == "target"),
        "module constant `target` should not be linearly checked in runtime fn: {lin:?}"
    );
}
