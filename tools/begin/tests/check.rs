use std::{fs, process::Command};

#[test]
fn check_reloads_sources_and_reports_all_type_errors_without_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.gin");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_begin"))
            .current_dir(dir.path())
            .env("GIN_HOME", home.path())
            .arg("check")
            .output()
            .unwrap()
    };
    fs::write(&source, "main: 1\n").unwrap();
    let valid = run();
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    fs::write(&source, "first: missing_one\nsecond: missing_two\n").unwrap();
    let invalid = run();
    let errors = String::from_utf8_lossy(&invalid.stderr);
    assert_eq!(invalid.status.code(), Some(1), "{errors}");
    for expected in [
        "missing_one",
        "missing_two",
        "main.gin",
        "type-unknown-symbol",
    ] {
        assert!(errors.contains(expected), "missing {expected}: {errors}");
    }
    fs::write(&source, "main: 2\n").unwrap();
    assert!(run().status.success());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(!home.path().join("cache/mods").exists());
}

#[test]
fn check_reports_parse_and_input_failures() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("broken.gin"), "main:\n").unwrap();
    for (path, expected) in [("broken.gin", "parse"), ("missing.gin", "failed to read")] {
        let output = Command::new(env!("CARGO_BIN_EXE_begin"))
            .current_dir(dir.path())
            .args(["check", path])
            .output()
            .unwrap();
        let errors = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{errors}");
        assert!(errors.contains(expected), "{errors}");
    }
}

#[test]
fn check_from_subdirectory_checks_entire_package() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("flask.jsonc"),
        r#"{"name":"fixture","version":"1.0.0","authors":[]}"#,
    )
    .unwrap();
    fs::write(dir.path().join("lib.gin"), "value: missing_root_symbol\n").unwrap();
    let child = dir.path().join("child");
    fs::create_dir(&child).unwrap();
    fs::write(child.join("valid.gin"), "other: 1\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(child)
        .arg("check")
        .output()
        .unwrap();
    let errors = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{errors}");
    assert!(errors.contains("missing_root_symbol"), "{errors}");
}

#[test]
fn explicit_package_path_resolves_dependencies_relative_to_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("app");
    let dependency = dir.path().join("dependency");
    fs::create_dir(&package).unwrap();
    fs::create_dir(&dependency).unwrap();
    fs::write(package.join("flask.jsonc"), r#"{"name":"fixture","version":"1.0.0","authors":[],"dependencies":{"support":{"path":"../dependency"}}}"#).unwrap();
    fs::write(
        dependency.join("flask.jsonc"),
        r#"{"name":"support","version":"1.0.0","authors":[]}"#,
    )
    .unwrap();
    fs::write(dependency.join("lib.gin"), "Thing is ()\n").unwrap();
    fs::write(
        package.join("main.gin"),
        "use support Thing\nvalue: Thing\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(dir.path())
        .args(["check", "app"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn json_check_is_machine_readable_and_limits_display_without_hiding_failure() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.gin"),
        "first: missing_one\nsecond: missing_two\n",
    )
    .unwrap();
    for limit in [0, 1, 10] {
        let output = Command::new(env!("CARGO_BIN_EXE_begin"))
            .current_dir(dir.path())
            .args([
                "check",
                "--message-format",
                "json",
                "--max-errors",
                &limit.to_string(),
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let records: Vec<serde_json::Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let summary = records.last().unwrap();
        assert_eq!(summary["reason"], "check-finished");
        assert_eq!(summary["success"], false);
        assert_eq!(summary["errors"], 2);
        assert_eq!(summary["omitted_errors"], 2_usize.saturating_sub(limit));
        assert_eq!(records.len(), limit.min(2) + 1);
        if limit > 0 {
            let diagnostic = &records[0];
            assert_eq!(diagnostic["code"], "type-unknown-symbol");
            assert_eq!(diagnostic["severity"], "error");
            assert_eq!(diagnostic["range"]["start"]["line"], 1);
            assert_eq!(diagnostic["range"]["start"]["column"], 8);
            assert_eq!(diagnostic["range"]["byte_start"], 7);
            assert!(diagnostic["file"].as_str().unwrap().ends_with("main.gin"));
            assert_eq!(diagnostic["dependency"], false);
        }
    }
}

#[test]
fn check_formats_success_input_errors_and_plain_text() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("main.gin"), "main: 1\n").unwrap();
    for (file, success) in [("main.gin", true), ("missing.gin", false)] {
        let output = Command::new(env!("CARGO_BIN_EXE_begin"))
            .current_dir(dir.path())
            .args(["check", file, "--message-format", "json"])
            .output()
            .unwrap();
        assert_eq!(output.status.success(), success);
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).unwrap();
        let records: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.last().unwrap()["success"], success);
    }
    fs::write(dir.path().join("main.gin"), "main: missing\n").unwrap();
    for format in ["human", "short"] {
        let output = Command::new(env!("CARGO_BIN_EXE_begin"))
            .current_dir(dir.path())
            .args(["check", "--message-format", format])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let text = String::from_utf8(output.stderr).unwrap();
        assert!(!text.contains('\u{1b}'), "{text}");
        assert!(text.contains("Check failed: 1 errors"), "{text}");
        if format == "short" {
            assert!(
                text.contains("main.gin:1:7: error[type-unknown-symbol]"),
                "{text}"
            );
        }
    }
}

#[test]
fn json_parse_errors_include_locations_and_dependency_origin() {
    let dir = tempfile::tempdir().unwrap();
    let dep = dir.path().join("dependency");
    fs::create_dir(&dep).unwrap();
    fs::write(
        dep.join("flask.jsonc"),
        r#"{"name":"support","version":"1.0.0","authors":[]}"#,
    )
    .unwrap();
    fs::write(
        dep.join("lib.gin"),
        "Thing is ()\nbroken: missing_dependency_symbol\n",
    )
    .unwrap();
    fs::write(dir.path().join("flask.jsonc"), serde_json::to_vec(&serde_json::json!({"name":"app", "version":"1.0.0", "authors":[], "dependencies":{"support":{"path":"dependency"}}})).unwrap()).unwrap();
    fs::write(
        dir.path().join("main.gin"),
        "use support Thing\nvalue: Thing\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(dir.path())
        .args(["check", "--message-format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        records
            .iter()
            .any(|r| r["code"] == "type-unknown-symbol" && r["dependency"] == true),
        "{text}"
    );
    fs::write(dir.path().join("main.gin"), "main:\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(dir.path())
        .args(["check", "--message-format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        records
            .iter()
            .any(|r| r["code"] == "parse-expected-expression" && r["range"]["start"]["line"] == 1),
        "{text}"
    );
}
