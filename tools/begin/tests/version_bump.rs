use std::fs;
use std::process::{Command, Output};

fn run(
    root: &std::path::Path,
    cache: &std::path::Path,
    old: &std::path::Path,
    check: bool,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_begin"));
    command
        .current_dir(root)
        .env("GIN_HOME", cache)
        .args(["version", "bump", "--against"])
        .arg(old);
    if check {
        command.arg("--check");
    }
    command.output().unwrap()
}

#[test]
fn unavailable_interface_emission_leaves_the_version_and_baseline_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let raw = "-- keep this\n#settings\nmodule: 'pkg'\nversion: 1.2.3\n#dependencies\n";
    fs::write(root.path().join("mod.flask"), raw).unwrap();
    fs::write(root.path().join("main.gin"), "ping(): return\n").unwrap();
    let baseline = interface::PublicInterface::from_subject_and_declarations(
        interface::PackageInstanceId {
            package: "pkg".into(),
            version: "1.2.3".into(),
            source: "fixture".into(),
            instance: "baseline".into(),
        },
        Vec::new(),
    )
    .canonical_bytes();
    let old = root.path().join("baseline.ginif");
    fs::write(&old, &baseline).unwrap();
    for check in [false, true] {
        let output = run(root.path(), cache.path(), &old, check);
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("package interface bundle emission is not implemented"),
            "{output:?}"
        );
        assert_eq!(fs::read(&old).unwrap(), baseline);
        assert_eq!(
            fs::read_to_string(root.path().join("mod.flask")).unwrap(),
            raw
        );
    }
    assert!(!root.path().join("target").exists());
}
