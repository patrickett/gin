use std::process::Command;

#[test]
fn build_status_uses_stderr_and_timings_are_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("input.gin"),
        "Word is in 0 to 255\nidentity(a Word) Word: a\n",
    )
    .unwrap();
    for extra in [None, Some("--timings"), Some("--quiet")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
        command
            .current_dir(dir.path())
            .env("GIN_HOME", dir.path().join("home"))
            .env_remove("GINC_TIMINGS")
            .env("NO_COLOR", "1")
            .args([
                "input.gin",
                "--emit",
                "obj",
                "--target",
                &test_fixtures::host_target_triple(),
                "-o",
                "output.o",
            ]);
        if let Some(extra) = extra {
            command.arg(extra);
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(output.stdout.is_empty());
        assert_eq!(stderr.contains("Compiling"), extra != Some("--quiet"));
        assert_eq!(stderr.contains("Finished"), extra != Some("--quiet"));
        assert_eq!(stderr.contains("Timings"), extra == Some("--timings"));
        assert!(!stderr.contains('\u{1b}'));
        if extra == Some("--timings") {
            assert!(stderr.contains("front-end"));
            assert!(stderr.contains("Object cache:"));
            assert!(stderr.contains('%'));
        }
    }
}

#[test]
fn failed_build_never_reports_finished_and_quiet_keeps_errors() {
    let dir = tempfile::tempdir().unwrap();
    for quiet in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
        command.current_dir(dir.path()).arg("missing.gin");
        if quiet {
            command.arg("--quiet");
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(stderr.contains("error:"));
        assert!(!stderr.contains("Finished"));
        assert_eq!(stderr.contains("Failed"), !quiet);
    }
}
