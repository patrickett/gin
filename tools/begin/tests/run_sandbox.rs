#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn run_compiles_into_user_cache_and_executes_in_sandbox() {
    let source = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("mod.flask"),
        format!(
            "#settings\nmodule: 'app'\nversion: 1.0.0\ntarget: '{}'\n",
            test_fixtures::host_target_triple()
        ),
    )
    .unwrap();
    std::fs::write(source.path().join("main.gin"), "main:\n    return 0\n").unwrap();
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
        source.path().join("mod.flask"),
        r#"#settings
module: "app"
version: 1.0.0

#dependencies
"#,
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
