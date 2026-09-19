#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn run_compiles_into_user_cache_and_executes_in_sandbox() {
    let source = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("flask.jsonc"),
        serde_json::to_vec(&serde_json::json!({"name":"app","version":"1.0.0","authors":[],"target":test_fixtures::host_target_triple()})).unwrap(),
    )
    .unwrap();
    std::fs::write(source.path().join("main.gin"), "main:\nreturn 0\n").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(source.path())
        .env("GIN_HOME", home.path())
        .arg("run")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Running"),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_dir(source.path()).unwrap().count(), 2);
    assert!(home.path().join("cache/mods").is_dir());
}

#[test]
fn run_reports_a_failed_compilation_as_failure() {
    let source = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("flask.jsonc"),
        r#"{"name":"app","version":"1.0.0","authors":[]}"#,
    )
    .unwrap();
    std::fs::write(source.path().join("main.gin"), "main:\n").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(source.path())
        .env("GIN_HOME", home.path())
        .arg("run")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Running"));
}
