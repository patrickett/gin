use std::process::Command;

#[test]
fn build_forwards_timings_and_release_and_reports_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("flask.jsonc"),
        format!(
            r#"{{"name":"fixture","version":"0.1.0","authors":[],"target":"{}"}}"#,
            test_fixtures::host_target_triple()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("lib.gin"),
        "Word is in 0 to 255\nidentity(a Word) Word: a\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(dir.path())
        .env("GIN_HOME", dir.path().join("home"))
        .env_remove("GINC_TIMINGS")
        .env("NO_COLOR", "1")
        .args(["build", "--release", "--timings"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(output.stdout.is_empty());
    for expected in [
        "Compiling",
        "fixture v0.1.0",
        "Finished",
        "release [optimized]",
        "Artifact",
        "Timings",
        "front-end",
    ] {
        assert!(stderr.contains(expected), "missing {expected}: {stderr}");
    }
    assert!(!stderr.contains('\u{1b}'));
    for quiet in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_begin"));
        command
            .current_dir(dir.path())
            .env("GIN_HOME", dir.path().join("home"))
            .env_remove("GINC_TIMINGS")
            .arg("build");
        if quiet {
            command.arg("--quiet");
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(output.stdout.is_empty());
        assert!(!stderr.contains("Timings"));
        assert_eq!(stderr.contains("Finished"), !quiet);
    }
}

#[test]
fn build_failures_exit_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_begin"))
            .current_dir(dir.path())
            .args(["build", "missing.gin"])
            .output()
            .unwrap()
    };
    let missing_manifest = run();
    assert!(!missing_manifest.status.success());
    assert!(String::from_utf8_lossy(&missing_manifest.stderr).contains("flask.jsonc"));
    std::fs::write(
        dir.path().join("flask.jsonc"),
        r#"{"name":"fixture","version":"0.1.0","authors":[]}"#,
    )
    .unwrap();
    let missing_source = run();
    assert!(!missing_source.status.success());
    assert!(String::from_utf8_lossy(&missing_source.stderr).contains("path does not exist"));
    assert!(!String::from_utf8_lossy(&missing_source.stderr).contains("Finished"));
    std::fs::write(dir.path().join("broken.gin"), "main:\n").unwrap();
    let invalid = Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(dir.path())
        .args(["build", "broken.gin", "--quiet"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(stderr.contains("error:"));
    assert!(!stderr.contains("Finished"));
}
