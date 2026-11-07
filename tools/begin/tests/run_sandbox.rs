#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn run_compiles_into_package_target_and_executes_new_entry_in_sandbox() {
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
    std::fs::write(
        source.path().join("application.gin"),
        "#entry\nstart() in 0 to 255: return 17\n",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(source.path())
        .env("GIN_HOME", home.path())
        .arg("run")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(17),
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
    assert!(source.path().join("target/debug/app").is_file());
    assert!(!home.path().join("cache/mods").exists());
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
    std::fs::write(
        source.path().join("main.gin"),
        "#entry\nstart() in 0 to 1: return 5\n",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(source.path())
        .env("GIN_HOME", home.path())
        .arg("run")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Running"));
}
