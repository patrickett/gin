//! A package that depends on another by path builds and runs through the new stack.
//!
//! `app` names `lib` in its manifest (`lib: "../lib"`); its import `use lib.util five` loads
//! the module of that package, which is lowered to its own object and linked in.
use std::path::Path;
use std::process::Command;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn workspace() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "lib/mod.flask",
        "#settings\nmodule: \"lib\"\nversion: 0.1.0\n",
    );
    write(root, "lib/util/util.gin", "five() in 0 to 16: return 5\n");
    write(
        root,
        "app/mod.flask",
        "#settings\nmodule: \"app\"\nversion: 0.1.0\n\n#dependencies\nlib: \"../lib\"\n",
    );
    write(
        root,
        "app/main.gin",
        "use lib.util five\n#entry\nstart() in 0 to 255: return five()\n",
    );
    directory
}

fn ginc(arguments: &[&str], package: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(arguments)
        .arg(package)
        .output()
        .unwrap()
}

#[test]
fn a_package_with_a_path_dependency_checks() {
    let directory = workspace();
    let output = ginc(&["--emit", "check"], &directory.path().join("app"));
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn the_dependencys_module_is_lowered_and_linked() {
    let directory = workspace();
    let program = directory.path().join("program");
    let output = ginc(
        &["--emit", "exe", "--output", program.to_str().unwrap()],
        &directory.path().join("app"),
    );
    assert!(output.status.success(), "{output:?}");
    let status = Command::new(&program).status().unwrap();
    assert_eq!(status.code(), Some(5));
}

#[test]
fn each_module_of_both_packages_is_its_own_mlir_module() {
    let directory = workspace();
    let output = ginc(&["--emit", "mlir"], &directory.path().join("app"));
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    // The dependency's module is lowered under its qualified name...
    assert!(text.contains("func.func @\"lib.util.five\""), "{text}");
    // ...and the call to it, whose arguments are all known, is already its value.
    assert!(!text.contains("func.call @\"lib.util.five\""), "{text}");
    assert!(text.contains("arith.constant 5"), "{text}");
}

#[test]
fn an_unreachable_dependency_is_reported_not_skipped() {
    let directory = workspace();
    std::fs::remove_dir_all(directory.path().join("lib")).unwrap();
    let output = ginc(&["--emit", "check"], &directory.path().join("app"));
    assert!(!output.status.success(), "{output:?}");
}
