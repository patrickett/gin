//! Native builds on the new stack: profiles, `--fast-llvm`, `--emit obj`, and where an
//! artifact goes when `--output` is not given.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const MANIFEST: &str = "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n";
const PROGRAM: &str = "#entry\nstart():\n    return\n";

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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn repository(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

#[test]
fn the_default_output_is_under_target_by_profile_and_module_name() {
    for (flags, profile) in [
        (&[][..], "debug"),
        (&["--profile", "release"][..], "release"),
    ] {
        let directory = package(&[("main.gin", PROGRAM)]);
        let output = ginc()
            .args(["--emit", "exe"])
            .args(flags)
            .arg(directory.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        let artifact = directory.path().join("target").join(profile).join("app");
        assert!(artifact.is_file(), "{profile}: {}", stderr(&output));
        assert!(
            stderr(&output).contains(&format!("Artifact {}", artifact.display())),
            "{}",
            stderr(&output)
        );
        assert!(Command::new(&artifact).status().unwrap().success());
    }
}

#[test]
fn a_file_of_the_root_module_builds_the_package_at_the_same_default_output() {
    let directory = package(&[("main.gin", PROGRAM)]);
    let output = ginc()
        .args(["--emit", "exe"])
        .arg(directory.path().join("main.gin"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(directory.path().join("target/debug/app").is_file());
}

#[test]
fn a_module_folder_with_an_entry_builds_under_its_own_name() {
    let directory = package(&[("tool/tool.gin", PROGRAM)]);
    let output = ginc()
        .args(["--emit", "exe"])
        .arg(directory.path().join("tool"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    // The package root is a library with no entry; the selected module is the program.
    assert!(directory.path().join("target/debug/tool").is_file());
}

#[test]
fn an_explicit_output_wins_and_leaves_no_target_directory() {
    let directory = package(&[("main.gin", PROGRAM)]);
    let program = directory.path().join("elsewhere");
    let output = ginc()
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(program.is_file());
    assert!(!directory.path().join("target").exists());
}

/// D1: a root with `#entry` is built by default, a library is only checked.
#[test]
fn the_default_action_builds_a_program_and_only_checks_a_library() {
    let program = package(&[("main.gin", PROGRAM)]);
    let output = ginc()
        .args(["--emit", "auto"])
        .arg(program.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(program.path().join("target/debug/app").is_file());
    let library = package(&[("main.gin", "one() in 0 to 1: return 1\n")]);
    let output = ginc()
        .args(["--emit", "auto"])
        .arg(library.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!library.path().join("target").exists());
}

#[test]
fn building_a_library_as_an_executable_says_there_is_no_entry() {
    let library = package(&[("main.gin", "one() in 0 to 1: return 1\n")]);
    let output = ginc()
        .args(["--emit", "exe"])
        .arg(library.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("error: package failed: the root module has no `#entry`"),
        "{}",
        stderr(&output)
    );
    assert!(!library.path().join("target/debug/app").exists());
}

#[test]
fn status_names_the_profile_and_the_optimization() {
    let directory = package(&[("main.gin", PROGRAM)]);
    let cases: [(&[&str], &str); 3] = [
        (&[], "debug [unoptimized]"),
        (&["--profile", "release"], "release [optimized]"),
        (
            &["--profile", "release", "--fast-llvm"],
            "release [fast LLVM]",
        ),
    ];
    for (flags, expected) in cases {
        let output = ginc()
            .args(["--emit", "exe"])
            .args(flags)
            .arg(directory.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(
            stderr(&output).contains(expected),
            "{expected}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn emit_obj_writes_one_object_per_module_and_links_nothing() {
    let directory = package(&[
        (
            "main.gin",
            "use app.util five\n#entry\nstart():\n    return\n",
        ),
        ("util/util.gin", "five() in 0 to 16: return 5\n"),
    ]);
    let output = ginc()
        .args(["--emit", "obj"])
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let objects = directory.path().join("target/debug/app.objects");
    let mut names: Vec<_> = std::fs::read_dir(&objects)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["app.o", "app.util.o"]);
    assert!(!directory.path().join("target/debug/app").exists());
    // An explicit directory is used as given, in either profile.
    let chosen = directory.path().join("objects");
    let output = ginc()
        .args(["--emit", "obj", "--profile", "release", "--output"])
        .arg(&chosen)
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(chosen.join("app.o").is_file() && chosen.join("app.util.o").is_file());
}

#[test]
fn a_library_also_yields_objects() {
    let library = package(&[("a/a.gin", "one() in 0 to 1: return 1\n")]);
    let output = ginc()
        .args(["--emit", "obj"])
        .arg(library.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        library
            .path()
            .join("target/debug/library.objects/app.a.o")
            .is_file()
    );
}

struct Case {
    stdin: String,
    stdout: String,
}

fn cases_of(name: &str) -> Vec<Case> {
    let text = std::fs::read_to_string(repository(&format!(
        "modules/popular_problems/{name}/cases.json"
    )))
    .unwrap();
    serde_json::from_str::<serde_json::Value>(&text)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|case| Case {
            stdin: case["stdin"].as_str().unwrap().to_owned(),
            stdout: case["stdout"].as_str().unwrap().to_owned(),
        })
        .collect()
}

/// Build the corpus package itself (nothing is written beside it) and run every case.
fn check_corpus_program(name: &str, flags: &[&str]) {
    let scratch = tempfile::tempdir().unwrap();
    let program = scratch.path().join("program");
    let built = ginc()
        .args(["--emit", "exe", "--quiet", "--output"])
        .arg(&program)
        .args(flags)
        .arg(repository(&format!("modules/popular_problems/{name}")))
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{name} {flags:?}:\n{}",
        stderr(&built)
    );
    for case in cases_of(name) {
        let mut child = Command::new(&program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(case.stdin.as_bytes());
        let done = child.wait_with_output().unwrap();
        assert_eq!(
            (
                String::from_utf8_lossy(&done.stdout).into_owned(),
                done.status.code()
            ),
            (case.stdout.clone(), Some(0)),
            "{name} {flags:?} on {:?}",
            case.stdin
        );
    }
}

/// Release must run programs correctly, with and without the faster LLVM path.
#[test]
fn release_builds_of_corpus_programs_match_their_cases() {
    for name in ["fizzbuzz", "digital_root"] {
        check_corpus_program(name, &["--profile", "release"]);
    }
}

#[test]
fn fast_llvm_builds_of_a_corpus_program_match_its_cases() {
    check_corpus_program("fizzbuzz", &["--profile", "release", "--fast-llvm"]);
    check_corpus_program("digital_root", &["--fast-llvm"]);
}
