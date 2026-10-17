use flask::{DependencyKind, FlaskConfig, FlaskConfigHandle};

const MANIFEST: &str = "-- package\n#settings\nmodule: 'app'\nversion: 0.1.0\ntarget: 'library'\n\n#dependencies\nhttp: 1.0.0\nhttp2: http@2.1.0\ncore: '../core' -- local\n";

#[test]
fn parses_typed_values_aliases_paths_and_comments() {
    let config = FlaskConfig::parse_manifest(MANIFEST).unwrap();
    assert!(config.minimum_version_selection);
    assert!(config.resolution.is_none());
    assert_eq!(config.name(), "app");
    assert_eq!(config.version(), "0.1.0");
    let minimums = config.registry_minimum_requirements().unwrap();
    assert_eq!(minimums["http"].version.to_string(), "1.0.0");
    assert_eq!(minimums["http2"].name, "http");
    assert_eq!(minimums["http2"].version.major, 2);
    assert!(
        matches!(&config.dependencies()["core"].kind, DependencyKind::Path { path } if path == "../core")
    );
    let formatted = config.format_manifest().unwrap();
    let restored = FlaskConfig::parse_manifest(&formatted).unwrap();
    assert_eq!(
        serde_json::to_value(config).unwrap(),
        serde_json::to_value(restored).unwrap()
    );
}

#[test]
fn rejects_ambiguous_invalid_and_unsupported_syntax_with_line_numbers() {
    for (from, to, line) in [
        ("module: 'app'", "module: app", 3),
        ("module: 'app'", "module: 'app' // comment", 3),
        ("module: 'app'", "module: 'app' /* comment */", 3),
        ("version: 0.1.0", "version: '0.1.0'", 4),
        ("target: 'library'", "target: library", 5),
        ("http: 1.0.0", "http: ^1.0.0", 8),
        ("http: 1.0.0", "http: 0.1.0", 8),
        ("http: 1.0.0", "http: 1.0.0-beta", 8),
        ("http: 1.0.0", "http: 1.0.0\nhttp: 1.1.0", 9),
        ("#dependencies", "#unknown", 7),
        ("#dependencies", "#settings", 7),
        ("core: '../core'", "core: '/core'", 10),
        ("core: '../core'", "core: '../core", 10),
        ("target: 'library'", "unknown: 'value'", 5),
    ] {
        let error = FlaskConfig::parse_manifest(&MANIFEST.replace(from, to)).unwrap_err();
        assert_eq!(error.line, line, "{from} -> {to}: {error}");
    }
    assert!(FlaskConfig::parse_manifest("#dependencies\n").is_err());
    assert!(FlaskConfig::parse_manifest(&" ".repeat(1024 * 1024 + 1)).is_err());
}

#[test]
fn quoted_comment_markers_and_empty_dependencies_are_valid() {
    let config = FlaskConfig::parse_manifest(
        "#settings\nmodule: 'app'\nversion: 1.0.0\ndescription: 'before -- after' -- comment\n#dependencies\n"
    ).unwrap();
    assert_eq!(config.description(), Some("before -- after"));
    assert!(config.dependencies().is_empty());
    assert!(config.target().is_none());
}

#[test]
fn discovery_stops_at_invalid_manifests() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("mod.flask"), MANIFEST).unwrap();
    let child = root.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(child.join("mod.flask"), "invalid").unwrap();
    assert!(FlaskConfig::discover(&child).is_err());
    assert!(FlaskConfig::find_package_config(&child).is_none());
}

#[test]
fn atomic_edits_preserve_comments_and_reject_stale_versions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mod.flask");
    std::fs::write(&path, MANIFEST).unwrap();
    let handle = FlaskConfigHandle::load(root.path()).unwrap();
    handle.update_version("0.1.0", "0.1.1").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        MANIFEST.replace("version: 0.1.0", "version: 0.1.1")
    );
    assert!(handle.update_version("0.1.0", "0.1.2").is_err());
    let mut requirements = handle
        .read()
        .config
        .registry_minimum_requirements()
        .unwrap();
    requirements.get_mut("http").unwrap().version = semver::Version::new(1, 2, 0);
    handle.update_minimums(&requirements).unwrap();
    let edited = std::fs::read_to_string(&path).unwrap();
    assert!(edited.contains("http: 1.2.0"));
    assert!(edited.contains("core: '../core' -- local"));
    assert!(!edited.contains("resolution"));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn discovery_ignores_legacy_files_and_never_parses_json_as_a_manifest() {
    let root = tempfile::tempdir().unwrap();
    for name in ["flask.json", "flask.jsonc"] {
        std::fs::write(
            root.path().join(name),
            r#"{"name":"old","version":"1.0.0","authors":[]}"#,
        )
        .unwrap();
    }
    assert!(FlaskConfig::discover(root.path()).unwrap().is_none());
    std::fs::write(root.path().join("mod.flask"), MANIFEST).unwrap();
    let (config, found) = FlaskConfig::discover(root.path()).unwrap().unwrap();
    assert_eq!(config.name(), "app");
    assert_eq!(found, root.path());
    std::fs::write(root.path().join("mod.flask"), "{}").unwrap();
    assert!(FlaskConfig::discover(root.path()).is_err());
}

#[test]
fn strings_and_lists_handle_escapes_unicode_and_quoted_separators() {
    let config = FlaskConfig::parse_manifest(
        r#"#settings
module: 'app'
version: 1.0.0
description: 'Jane\'s "quoted" \\path\n\u263a -- /* //'
authors: ['Jane, Jo', "雪", '\uD83D\uDE00',]
keywords: []
"#,
    )
    .unwrap();
    assert_eq!(
        config.description(),
        Some("Jane's \"quoted\" \\path\n☺ -- /* //")
    );
    assert_eq!(
        config
            .authors()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["Jane, Jo", "雪", "😀"]
    );
    let restored = FlaskConfig::parse_manifest(&config.format_manifest().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(config).unwrap(),
        serde_json::to_value(restored).unwrap()
    );
}

#[test]
fn invalid_strings_and_lists_report_the_setting_line() {
    for value in [
        "['Jane' 'Jo']",
        "['Jane',,]",
        "[1]",
        "['Jane'] trailing",
        "['Jane]",
    ] {
        let raw = format!("#settings\nmodule: 'app'\nversion: 1.0.0\nauthors: {value}\n");
        assert_eq!(
            FlaskConfig::parse_manifest(&raw).unwrap_err().line,
            4,
            "{value}"
        );
    }
    for value in [r"'bad\q'", r"'bad\uD800'", "'first' 'second'"] {
        let raw = format!("#settings\nmodule: 'app'\nversion: 1.0.0\ndescription: {value}\n");
        assert_eq!(
            FlaskConfig::parse_manifest(&raw).unwrap_err().line,
            4,
            "{value}"
        );
    }
}
