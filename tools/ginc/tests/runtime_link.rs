//! User Gin definitions do not depend on an injected runtime C implementation.
use std::process::Command;

#[test]
fn local_runtime_named_definition_builds_and_runs_without_runtime_injection() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"app\"\nversion: 0.1.0\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("main.gin"),
        "runtime_print_int(value in 0 to 255) in 0 to 255: return value\n\
         #entry\nmain() in 0 to 255: return runtime_print_int(23)\n",
    )
    .unwrap();
    for profile in ["debug", "release"] {
        let executable = directory.path().join(format!("program-{profile}"));
        let compilation = Command::new(env!("CARGO_BIN_EXE_ginc"))
            .args(["--emit", "exe", "--profile", profile, "--target"])
            .arg(test_fixtures::host_target_triple())
            .arg("--output")
            .arg(&executable)
            .arg(directory.path())
            .output()
            .unwrap();
        assert!(compilation.status.success(), "{compilation:?}");
        let process = Command::new(&executable).output().unwrap();
        assert_eq!(process.status.code(), Some(23), "{process:?}");
        assert!(process.stdout.is_empty(), "{process:?}");
    }
}
