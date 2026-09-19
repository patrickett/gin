use flask::{FlaskConfig, FlaskConfigHandle, GinHome};
use semver::Version;

#[test]
fn cache_paths_are_external_and_separate_inputs_builds_profiles_and_targets() {
    let source = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let home = GinHome::new(cache.path().to_path_buf());
    let first = home
        .artifacts(source.path(), b"first", "debug", "aarch64-apple-darwin")
        .unwrap();
    assert!(first.starts_with(cache.path()));
    assert!(!first.starts_with(source.path()));
    assert_ne!(
        first,
        home.artifacts(source.path(), b"second", "debug", "aarch64-apple-darwin")
            .unwrap()
    );
    assert_ne!(
        first,
        home.artifacts(source.path(), b"first", "release", "aarch64-apple-darwin")
            .unwrap()
    );
    assert_ne!(
        first,
        home.artifacts(source.path(), b"first", "debug", "x86_64-unknown-linux-gnu")
            .unwrap()
    );
    assert!(
        home.artifacts(source.path(), b"first", "../outside", "library")
            .is_err()
    );
    assert!(
        home.dependency("../outside", &Version::new(1, 0, 0))
            .is_err()
    );
    assert_eq!(
        home.dependency("http", &Version::new(1, 0, 1)).unwrap(),
        cache.path().join("cache/deps/http@1.0.1")
    );
    assert_eq!(std::fs::read_dir(source.path()).unwrap().count(), 0);
}

#[test]
fn version_update_preserves_manifest_and_rejects_stale_versions() {
    let source = tempfile::tempdir().unwrap();
    let raw = "{\n // description stays\n name: 'pkg', version: '1.2.3', authors: [], dependencies: { http: { version: '^1' } }\n}\n";
    let path = source.path().join("flask.jsonc");
    std::fs::write(&path, raw).unwrap();
    let config = FlaskConfigHandle::load(source.path()).unwrap();
    config.update_version("1.2.3", "1.3.0").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        raw.replace("'1.2.3'", "\"1.3.0\"")
    );
    assert!(config.update_version("1.2.3", "2.0.0").is_err());
    assert_eq!(config.read().config.version, "1.3.0");
}

#[test]
fn embedded_resolution_round_trips_and_checks_requirements() {
    let raw = format!(
        r#"{{"name":"pkg","version":"1.0.0","authors":[],
        "dependencies":{{"http":{{"version":"^1"}}}},
        "resolution":{{"direct":{{"http":"http@1.0.1"}},"packages":{{"http@1.0.1":{{"name":"http","version":"1.0.1","sha256":"{}"}}}}}}
    }}"#,
        "0".repeat(64)
    );
    let config: FlaskConfig = json5::from_str(&raw).unwrap();
    config.validate_resolution().unwrap();
    let json = serde_json::to_string(&config).unwrap();
    let restored: FlaskConfig = json5::from_str(&json).unwrap();
    restored.validate_resolution().unwrap();
    let changed: FlaskConfig = json5::from_str(&raw.replace("^1", "^2")).unwrap();
    assert!(changed.validate_resolution().is_err());
}

#[test]
fn resolution_updates_preserve_requirements_and_other_comments() {
    let source = tempfile::tempdir().unwrap();
    let raw = "{ // keep\n name: 'pkg', version: '1.0.0', authors: [] // tail\n}\n";
    let path = source.path().join("flask.jsonc");
    std::fs::write(&path, raw).unwrap();
    let config = FlaskConfigHandle::load(source.path()).unwrap();
    config
        .update_resolution(flask::PackageResolution::default())
        .unwrap();
    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(updated.contains("// keep"));
    assert!(updated.contains("// tail"));
    config
        .update_resolution(flask::PackageResolution::default())
        .unwrap();
    assert_eq!(updated, std::fs::read_to_string(&path).unwrap());
    let parsed: FlaskConfig = json5::from_str(&updated).unwrap();
    parsed.validate_resolution().unwrap();
    assert!(parsed.resolution.is_some());
}
