//! Build status lines, `--quiet` and timing reports through the ordinary compiler CLI.
use std::path::Path;
use std::process::{Command, Output};

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

fn package(main: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("mod.flask"), MANIFEST).unwrap();
    std::fs::write(directory.path().join("main.gin"), main).unwrap();
    directory
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn report(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn phase_names(report: &serde_json::Value) -> Vec<String> {
    report["phases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|phase| phase["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn build_status_uses_stderr_and_timings_are_opt_in() {
    let directory = package(PROGRAM);
    for extra in [None, Some("--timings"), Some("--quiet")] {
        let mut command = ginc();
        command.args(["--emit", "exe"]).arg(directory.path());
        if let Some(extra) = extra {
            command.arg(extra);
        }
        let output = command.output().unwrap();
        let text = stderr(&output);
        assert!(output.status.success(), "{text}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            text.contains("Compiling app v0.1.0"),
            extra != Some("--quiet"),
            "{text}"
        );
        assert_eq!(
            text.contains("Finished"),
            extra != Some("--quiet"),
            "{text}"
        );
        assert_eq!(
            text.contains("Timings"),
            extra == Some("--timings"),
            "{text}"
        );
        assert!(!text.contains('\u{1b}'));
        if extra == Some("--timings") {
            for phase in ["load", "check", "lower", "object", "link", "other"] {
                assert!(text.contains(phase), "{phase}: {text}");
            }
            assert!(text.contains('%'));
            // The new stack has no object cache, so it claims no outcome for one.
            assert!(!text.contains("Object cache"), "{text}");
        }
    }
}

#[test]
fn the_environment_turns_timings_on_for_every_mode() {
    let directory = package(PROGRAM);
    for mode in [&["--emit", "check"][..], &["--emit", "mlir"]] {
        let output = ginc()
            .env("GINC_TIMINGS", "1")
            .args(mode)
            .arg(directory.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("Timings"),
            "{mode:?}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn a_failed_build_never_reports_finished_and_quiet_keeps_errors() {
    let directory = package("start() in 0 to 1: return 5\n#entry\n");
    for quiet in [false, true] {
        let mut command = ginc();
        command.args(["--emit", "exe"]).arg(directory.path());
        if quiet {
            command.arg("--quiet");
        }
        let output = command.output().unwrap();
        let text = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{text}");
        assert!(text.contains("flaw: ") || text.contains("error:"), "{text}");
        assert!(text.contains("Check failed"), "{text}");
        assert!(!text.contains("Finished"), "{text}");
        assert_eq!(text.contains("Failed after"), !quiet, "{text}");
    }
}

#[test]
fn a_failure_with_no_diagnostic_is_printed_once() {
    let directory = tempfile::tempdir().unwrap();
    let output = ginc()
        .args(["--emit", "exe"])
        .arg(directory.path())
        .output()
        .unwrap();
    let text = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert_eq!(text.matches("error: package failed").count(), 1, "{text}");
}

#[test]
fn plain_check_and_mlir_print_no_build_status() {
    let directory = package(PROGRAM);
    for mode in [&["--emit", "check"][..], &["--emit", "mlir"]] {
        let output = ginc().args(mode).arg(directory.path()).output().unwrap();
        let text = stderr(&output);
        assert!(output.status.success(), "{text}");
        assert!(
            !text.contains("Compiling") && !text.contains("Finished"),
            "{mode:?}: {text}"
        );
    }
}

#[test]
fn the_timing_report_is_json_with_the_new_phases_and_hides_status() {
    let directory = package(PROGRAM);
    let timings = directory.path().join("timings.json");
    let output = ginc()
        .args(["--emit", "exe", "--timings-json"])
        .arg(&timings)
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stderr(&output).contains("Compiling") && !stderr(&output).contains("Finished"));
    let report = report(&timings);
    assert_eq!(report["schema_version"], 1);
    assert!(report["object_cache_hit"].is_null());
    assert!(report["total_seconds"].as_f64().unwrap() > 0.0);
    let names = phase_names(&report);
    for phase in ["load", "check", "lower", "object", "link"] {
        assert!(names.iter().any(|name| name == phase), "{phase}: {names:?}");
    }
    // Phases come in the order the work is done.
    let position = |wanted: &str| names.iter().position(|name| name == wanted).unwrap();
    assert!(position("load") < position("check") && position("check") < position("lower"));
    assert!(position("lower") < position("object") && position("object") < position("link"));
}

#[test]
fn a_check_reports_only_load_and_check() {
    let directory = package(PROGRAM);
    let timings = directory.path().join("timings.json");
    let output = ginc()
        .args(["--emit", "check", "--timings-json"])
        .arg(&timings)
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(phase_names(&report(&timings)), ["load", "check"]);
}

#[test]
fn the_timing_report_survives_an_early_failure() {
    let directory = tempfile::tempdir().unwrap();
    let timings = directory.path().join("timings.json");
    let output = ginc()
        .args(["--emit", "check", "--timings-json"])
        .arg(&timings)
        .arg(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report = report(&timings);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(phase_names(&report)[0], "load");
    assert!(report["total_seconds"].as_f64().unwrap() >= 0.0);
}

#[test]
fn the_timing_report_covers_a_standalone_file_too() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("loose.gin");
    std::fs::write(&file, "identity(x in 0 to 10) in 0 to 10: return x\n").unwrap();
    let timings = directory.path().join("timings.json");
    let output = ginc()
        .args(["--emit", "mlir", "--output"])
        .arg(directory.path().join("out.mlir"))
        .arg("--timings-json")
        .arg(&timings)
        .arg(&file)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(phase_names(&report(&timings)), ["check", "lower"]);
}
