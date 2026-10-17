use flask::FlaskConfig;
use std::fs;

fn manifest(root: &std::path::Path, name: &str, dependencies: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("mod.flask"),
        format!("#settings\nmodule: '{name}'\nversion: 0.1.0\n#dependencies\n{dependencies}"),
    )
    .unwrap();
}

#[test]
fn reachable_local_requirements_are_combined_deterministically_and_cycles_terminate() {
    let tmp = tempfile::tempdir().unwrap();
    manifest(
        &tmp.path().join("app"),
        "app",
        "first: '../first'\nsecond: '../second'\n",
    );
    manifest(
        &tmp.path().join("first"),
        "first",
        "http: 1.2.0\napp: '../app'\n",
    );
    manifest(&tmp.path().join("second"), "second", "http: 1.4.0\n");
    let root = tmp.path().join("app");
    let (config, _) = FlaskConfig::discover(&root).unwrap().unwrap();
    let expected = config
        .with_workspace_requirements(&root)
        .unwrap()
        .registry_minimum_requirements()
        .unwrap();
    assert_eq!(expected.len(), 2);
    assert!(expected.values().any(|r| r.version.to_string() == "1.4.0"));
    for _ in 0..20 {
        let (config, _) = FlaskConfig::discover(&root).unwrap().unwrap();
        assert_eq!(
            config
                .with_workspace_requirements(&root)
                .unwrap()
                .registry_minimum_requirements()
                .unwrap(),
            expected
        );
    }
}

#[test]
fn conflicting_local_providers_and_missing_manifests_fail() {
    let tmp = tempfile::tempdir().unwrap();
    manifest(&tmp.path().join("app"), "app", "a: '../a'\nb: '../b'\n");
    manifest(&tmp.path().join("a"), "core", "");
    manifest(&tmp.path().join("b"), "core", "");
    let root = tmp.path().join("app");
    let (config, _) = FlaskConfig::discover(&root).unwrap().unwrap();
    assert!(
        config
            .with_workspace_requirements(&root)
            .unwrap_err()
            .to_string()
            .contains("conflicting local providers")
    );
    fs::remove_file(tmp.path().join("b/mod.flask")).unwrap();
    assert!(config.with_workspace_requirements(&root).is_err());
}

#[test]
fn uninstalled_local_dependencies_contribute_requirements_when_installed() {
    let tmp = tempfile::tempdir().unwrap();
    manifest(tmp.path(), "app", "dev: './dev'\n");
    let (config, _) = FlaskConfig::discover(tmp.path()).unwrap().unwrap();
    assert!(
        config
            .with_workspace_requirements(tmp.path())
            .unwrap()
            .registry_minimum_requirements()
            .unwrap()
            .is_empty()
    );
    manifest(&tmp.path().join("dev"), "dev", "http: 1.2.0\n");
    let requirements = config
        .with_workspace_requirements(tmp.path())
        .unwrap()
        .registry_minimum_requirements()
        .unwrap();
    assert_eq!(requirements.len(), 1);
    assert_eq!(
        requirements.values().next().unwrap().version.to_string(),
        "1.2.0"
    );
}
