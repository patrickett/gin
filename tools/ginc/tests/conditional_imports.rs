mod common;
use common::compile_exe;
use std::process::Command;

use ginc::cli::Profile;
use test_fixtures::TempPackage;

const MAIN: &str =
    include_str!("../../../crates/test-fixtures/fixtures/conditional_imports/main.gin");
const FLAGS: &str =
    include_str!("../../../crates/test-fixtures/fixtures/conditional_imports/build/flags.gin");
const MANIFEST: &str =
    include_str!("../../../crates/test-fixtures/fixtures/conditional_imports/mod.flask");
const NAMED: &str =
    include_str!("../../../crates/test-fixtures/fixtures/conditional_imports/named.gin");
const CROSS_FILE: &str =
    include_str!("../../../crates/test-fixtures/fixtures/conditional_imports/cross_file.gin");
const LAUNCHER: &str =
    include_str!("../../../crates/test-fixtures/fixtures/conditional_imports/workers/launcher.gin");
const RUNNER: &str =
    include_str!("../../../crates/test-fixtures/fixtures/conditional_imports/workers/runner.gin");

fn fixture(developer: bool, installed: bool) -> TempPackage {
    let package = TempPackage::new("conditional_imports_native");
    package.write("mod.flask", MANIFEST);
    package.write("main.gin", MAIN);
    package.write(
        "build/flags.gin",
        &FLAGS.replace(
            "developer := 0",
            if developer {
                "developer := 1"
            } else {
                "developer := 0"
            },
        ),
    );
    package.write("game/game.gin", "run: 7\n");
    if installed {
        package.write(
            "dev/mod.flask",
            r#"#settings
module: "dev"
version: 0.0.0

#dependencies
"#,
        );
        package.write("dev/tools/tools.gin", "show_wireframe: 42\n");
    }
    package
}

fn compile(
    package: &TempPackage,
    profile: Profile,
) -> (ginc::compile::CompileResult, std::path::PathBuf) {
    let output = package.join("program");
    let result = compile_exe(package.join("main.gin"), &output, profile);
    (result, output)
}

#[test]
fn release_build_runs_without_developer_tools_installed() {
    let package = fixture(false, false);
    assert!(!package.join("dev").exists());
    let (result, executable) = compile(&package, Profile::Release);
    assert!(result.is_success(), "{result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    let output = Command::new(executable).output().unwrap();
    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn developer_build_runs_the_imported_branch() {
    let package = fixture(true, true);
    let (result, executable) = compile(&package, Profile::Debug);
    assert!(result.is_success(), "{result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    let output = Command::new(executable).output().unwrap();
    assert_eq!(output.status.code(), Some(42));
}

#[test]
fn developer_build_requires_developer_tools() {
    let package = fixture(true, false);
    let (result, executable) = compile(&package, Profile::Debug);
    assert!(!result.is_success(), "{result:?}");
    assert!(!executable.exists());
}

fn call_examples() -> Vec<String> {
    let direct = CROSS_FILE.replace(
        "'./workers/launcher.gin' as launcher",
        "'./workers/runner.gin' as runner",
    );
    vec![
        NAMED.to_owned(),
        CROSS_FILE.to_owned(),
        direct.replace("launcher.launch", "runner.run"),
        direct.replace(
            "launcher.launch(release_code: 7, developer: env.enabled('developer'))",
            "runner.run(env.enabled('developer'), 7)",
        ),
    ]
}

fn call_fixture(source: &str, developer: bool, installed: bool) -> TempPackage {
    let package = fixture(developer, installed);
    package.write("main.gin", source);
    package.write("workers/launcher.gin", LAUNCHER);
    package.write("workers/runner.gin", RUNNER);
    package
}

#[test]
fn named_and_cross_file_release_builds_run_without_developer_tools() {
    for source in call_examples() {
        let package = call_fixture(&source, false, false);
        assert!(!package.join("dev").exists());
        let (result, executable) = compile(&package, Profile::Release);
        assert!(result.is_success(), "{result:?}\n{source}");
        assert!(!result.has_typecheck_diagnostics(), "{result:?}\n{source}");
        assert_eq!(
            Command::new(executable).output().unwrap().status.code(),
            Some(7),
            "{source}"
        );
    }
}

#[test]
fn named_and_cross_file_developer_builds_run_the_imported_branch() {
    for source in call_examples() {
        let package = call_fixture(&source, true, true);
        let (result, executable) = compile(&package, Profile::Debug);
        assert!(result.is_success(), "{result:?}\n{source}");
        assert!(!result.has_typecheck_diagnostics(), "{result:?}\n{source}");
        assert_eq!(
            Command::new(executable).output().unwrap().status.code(),
            Some(42),
            "{source}"
        );
    }
}

#[test]
fn named_and_cross_file_developer_builds_require_developer_tools() {
    for source in call_examples() {
        let package = call_fixture(&source, true, false);
        let (result, executable) = compile(&package, Profile::Debug);
        assert!(!result.is_success(), "{result:?}\n{source}");
        assert!(!executable.exists(), "{source}");
    }
}
