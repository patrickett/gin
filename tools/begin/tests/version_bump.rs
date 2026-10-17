use flask::GinHome;
use ginc::cli::{Args, Emit};
use ginc::compile::{CompileResult, GinCompiler};
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
fn bump_checks_updates_preserves_comments_and_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let raw = "-- keep this\n#settings\nmodule: 'pkg'\nversion: 1.2.3\n#dependencies\n";
    fs::write(root.path().join("mod.flask"), raw).unwrap();
    fs::write(root.path().join("main.gin"), "ping:\n    return\n").unwrap();
    let result = GinCompiler::compile(&mut Args {
        input: root.path().to_path_buf(),
        emit: Emit::Interface,
        gin_home: Some(GinHome::new(cache.path().to_path_buf())),
        ..Default::default()
    });
    let CompileResult::Success { output: Some(old) } = result else {
        panic!("{result:?}");
    };
    let check = run(root.path(), cache.path(), &old, true);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("1.2.4"));
    assert_eq!(
        fs::read_to_string(root.path().join("mod.flask")).unwrap(),
        raw
    );
    fs::write(
        root.path().join("main.gin"),
        "ping:\n    return\npong:\n    return\n",
    )
    .unwrap();
    let apply = run(root.path(), cache.path(), &old, false);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let expected = raw.replace("version: 1.2.3", "version: 1.3.0");
    assert_eq!(
        fs::read_to_string(root.path().join("mod.flask")).unwrap(),
        expected
    );
    let again = run(root.path(), cache.path(), &old, false);
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("mod.flask")).unwrap(),
        expected
    );
    fs::write(root.path().join("main.gin"), "main: @\n").unwrap();
    assert!(!run(root.path(), cache.path(), &old, false).status.success());
    assert_eq!(
        fs::read_to_string(root.path().join("mod.flask")).unwrap(),
        expected
    );
    assert!(!root.path().join("target").exists());
}
