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
    let raw = "-- description stays\n#settings\nmodule: 'pkg'\nversion: 1.2.3\n#dependencies\nhttp: 1.0.0\n";
    let path = source.path().join("mod.flask");
    std::fs::write(&path, raw).unwrap();
    let config = FlaskConfigHandle::load(source.path()).unwrap();
    config.update_version("1.2.3", "1.3.0").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        raw.replace("version: 1.2.3", "version: 1.3.0")
    );
    assert!(config.update_version("1.2.3", "2.0.0").is_err());
    assert_eq!(config.read().config.version, "1.3.0");
}

#[test]
fn save_round_trips_manifest_metadata_and_minimums() {
    let source = tempfile::tempdir().unwrap();
    let raw = "#settings\nmodule: 'pkg'\nversion: 1.0.0\nauthors: ['Jane', \"Jo\"]\ndescription: 'sample'\n#dependencies\nhttp: 1.2.0\ncore: '../core'\n";
    let path = source.path().join("mod.flask");
    std::fs::write(&path, raw).unwrap();
    let handle = FlaskConfigHandle::load(source.path()).unwrap();
    handle.save().unwrap();
    let saved = FlaskConfig::parse_manifest(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&handle.read().config).unwrap(),
        serde_json::to_value(saved).unwrap()
    );
    assert_eq!(std::fs::read_dir(source.path()).unwrap().count(), 1);
}
