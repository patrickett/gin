use std::{path::Path, process::Command};

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_begin"))
        .current_dir(root)
        .env("GIN_HOME", root.join("unused-home"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn minimum_commands_work_without_registry_state_for_local_only_packages() {
    let root = tempfile::tempdir().unwrap();
    let manifest = "-- keep\n#settings\nmodule: 'app'\nversion: 0.1.0\n#dependencies\n";
    std::fs::write(root.path().join("mod.flask"), manifest).unwrap();
    std::fs::create_dir(root.path().join("core")).unwrap();
    std::fs::write(
        root.path().join("core/mod.flask"),
        "#settings\nmodule: 'core'\nversion: 0.1.0\n",
    )
    .unwrap();
    std::fs::write(root.path().join("main.gin"), "Choice is Left or Right\n").unwrap();
    for args in [
        &["add", "./core"][..],
        &["fetch", "--offline"],
        &["update"],
        &["audit"],
        &["check"],
    ] {
        let result = run(root.path(), args);
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let updated = std::fs::read_to_string(root.path().join("mod.flask")).unwrap();
    assert!(updated.starts_with("-- keep\n"));
    assert!(updated.contains("core: \"./core\""));
    assert!(!updated.contains("resolution"));
    assert!(!root.path().join("unused-home").exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 3);
}
