//! What an input names on the new stack: a package, a folder or file inside one (that
//! package, with that module as the root), a library with an empty root, or a loose file.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MANIFEST: &str = "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n";
const BAD: &str = "bad() in 0 to 1: return 5\n";

fn ginc() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
    command
        .env_remove("GIN_PROFILE_JSON")
        .env_remove("GINC_TIMINGS")
        .env_remove("GINC_FAST_LLVM")
        .env("NO_COLOR", "1");
    command
}

fn package(files: &[(&str, &str)]) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("mod.flask"), MANIFEST).unwrap();
    for (path, text) in files {
        let path = directory.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    directory
}

fn check(input: &Path) -> Output {
    ginc()
        .args(["--emit", "check"])
        .arg(input)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn files(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn repository(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

#[test]
fn a_file_in_a_package_is_checked_with_its_package_imports_resolved() {
    let directory = package(&[
        ("util/util.gin", "one() in 0 to 1: return 1\n"),
        (
            "main.gin",
            "use app.util one\ntwo() in 0 to 1: return one()\n",
        ),
    ]);
    // Alone, the file could not see `app.util`; as a member of its package it can.
    let output = check(&directory.path().join("main.gin"));
    assert!(output.status.success(), "{}", stderr(&output));
    let output = check(directory.path());
    assert!(output.status.success(), "{}", stderr(&output));
}

#[test]
fn a_file_selects_its_own_module_as_the_root() {
    let directory = package(&[
        ("main.gin", BAD),
        ("util/util.gin", "one() in 0 to 1: return 1\n"),
        ("util/more.gin", "two() in 0 to 1: return 0\n"),
    ]);
    // The root module is bad; the `util` module, named by a file or by its folder, is not.
    assert_eq!(check(directory.path()).status.code(), Some(1));
    assert_eq!(
        check(&directory.path().join("main.gin")).status.code(),
        Some(1)
    );
    for input in [
        directory.path().join("util/util.gin"),
        directory.path().join("util"),
    ] {
        let output = check(&input);
        assert!(output.status.success(), "{input:?}: {}", stderr(&output));
    }
}

#[test]
fn a_file_selects_the_whole_module_not_only_itself() {
    let directory = package(&[
        ("util/good.gin", "one() in 0 to 1: return 1\n"),
        ("util/bad.gin", BAD),
    ]);
    let output = check(&directory.path().join("util/good.gin"));
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("bad.gin"), "{}", stderr(&output));
}

#[test]
fn a_loose_file_outside_any_package_is_still_checked_alone() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("loose.gin");
    std::fs::write(&path, "identity(x in 0 to 10) in 0 to 10: return x\n").unwrap();
    let output = check(&path);
    assert!(output.status.success(), "{}", stderr(&output));
    std::fs::write(&path, BAD).unwrap();
    assert_eq!(check(&path).status.code(), Some(1));
}

#[test]
fn a_folder_with_no_package_around_it_is_refused_and_a_non_gin_file_too() {
    let directory = tempfile::tempdir().unwrap();
    let output = check(directory.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("mod.flask"), "{}", stderr(&output));
    let text = directory.path().join("notes.txt");
    std::fs::write(&text, "").unwrap();
    let output = check(&text);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("expected a package directory or an existing .gin file"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        check(&directory.path().join("missing.gin")).status.code(),
        Some(1)
    );
}

#[test]
fn a_library_with_an_empty_root_checks_all_its_modules() {
    let directory = package(&[
        ("a/a.gin", "one() in 0 to 1: return 1\n"),
        ("b/b.gin", "two() in 0 to 1: return 0\n"),
    ]);
    let output = check(directory.path());
    assert!(output.status.success(), "{}", stderr(&output));
    // A flaw in any module fails the library.
    std::fs::write(directory.path().join("b/b.gin"), BAD).unwrap();
    let output = check(directory.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("b.gin"), "{}", stderr(&output));
}

#[test]
fn the_examples_package_checks() {
    let output = check(&repository("modules/examples"));
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("Check passed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_program_file_in_the_corpus_checks_as_its_package() {
    let hello = repository("modules/popular_problems/hello_world");
    for input in [hello.clone(), hello.join("main.gin")] {
        let output = check(&input);
        assert!(output.status.success(), "{input:?}: {}", stderr(&output));
    }
}

/// D1: with no emission asked for, a library is checked and nothing is written.
#[test]
fn the_default_action_of_a_library_is_to_check_it() {
    let directory = package(&[("a/a.gin", "one() in 0 to 1: return 1\n")]);
    let output = ginc().arg(directory.path()).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("Check passed"),
        "{}",
        stderr(&output)
    );
    assert_eq!(files(directory.path()), ["a", "mod.flask"]);
}

#[test]
fn a_loose_file_cannot_be_built_but_is_checked_by_default() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("loose.gin");
    std::fs::write(&path, "identity(x in 0 to 10) in 0 to 10: return x\n").unwrap();
    let output = ginc().arg(&path).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let output = ginc().args(["--emit", "exe"]).arg(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("outside any package"),
        "{}",
        stderr(&output)
    );
    assert_eq!(files(directory.path()), ["loose.gin"]);
}
