use flask::{CompileTarget, FlaskConfig, TargetArch, TargetTriple};

#[test]
fn parse_linux_triple() {
    let t = TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap();
    assert_eq!(t.arch, TargetArch::X86_64);
    assert_eq!(t.os.as_str(), "linux");
}

#[test]
fn parse_wasm_triple() {
    let t = TargetTriple::parse("wasm32-unknown-unknown").unwrap();
    assert_eq!(t.arch, TargetArch::Wasm32);
    assert_eq!(t.os.as_str(), "unknown");
}

#[test]
fn reject_bare_arch() {
    assert!(TargetTriple::parse("x86_64").is_err());
}

#[test]
fn deny_unknown_targets_field() {
    let raw = r#"{"name":"x","version":"0","authors":[],"targets":[]}"#;
    let err = json5::from_str::<FlaskConfig>(raw).unwrap_err();
    assert!(err.to_string().contains("targets"));
}

#[test]
fn cli_target_overrides_flask_target() {
    let mut config = FlaskConfig::new("x".into(), "0".into());
    config.target = Some("x86_64-unknown-linux-gnu".into());
    let target = CompileTarget::resolve(&config, Some("wasm32-unknown-unknown"))
        .expect("CLI target override");
    assert_eq!(target.arch_literal(), Some("wasm32"));
    assert_eq!(target.triple_raw(), Some("wasm32-unknown-unknown"));
}

#[test]
fn flask_target_is_used_without_cli_override() {
    let mut config = FlaskConfig::new("x".into(), "0".into());
    config.target = Some("aarch64-apple-darwin".into());
    let target = CompileTarget::resolve(&config, None).expect("Flask target");
    assert_eq!(target.arch_literal(), Some("arm64"));
    assert_eq!(target.os_literal(), Some("macOS"));
}

#[test]
fn absent_target_remains_targetless() {
    let config = FlaskConfig::new("x".into(), "0".into());
    assert_eq!(
        CompileTarget::resolve(&config, None).expect("targetless library"),
        CompileTarget::Library
    );
}
