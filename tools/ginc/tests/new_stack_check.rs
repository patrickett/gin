//! Real compiler binary checks only: these tests do not emit or execute Gin code.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use diagnostic::DiagnosticPathExt;
use ginc::cli::{Args, Emit};
use ginc::compile::{CompileResult, GinCompiler};
use ginc::reporting::{CheckOptions, MessageFormat};
use serde_json::Value;

const VALID: &str = include_str!("../../../crates/typecheck/examples/source-echo.gin");
const INVALID: &str =
    include_str!("../../../crates/typecheck/examples/source-echo-invalid.gin");

fn fixture(text: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, text).unwrap();
    (directory, path)
}

fn command(path: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
    command
        .args(["--emit", "check"])
        .arg("--message-format=json")
        .arg(path)
        .env_remove("GIN_PROFILE_JSON")
        .env_remove("GINC_TIMINGS")
        .env_remove("GINC_FAST_LLVM");
    command
}

fn messages(output: &Output) -> Vec<Value> {
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    stdout
        .lines()
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}\n{output:?}"))
        })
        .collect()
}

fn finished(messages: &[Value]) -> &Value {
    let summaries: Vec<_> = messages
        .iter()
        .filter(|message| message["reason"] == "check-finished")
        .collect();
    assert_eq!(summaries.len(), 1, "{messages:?}");
    summaries[0]
}

fn diagnostic<'a>(messages: &'a [Value], code: &str) -> &'a Value {
    messages
        .iter()
        .find(|message| message["reason"] == "diagnostic" && message["code"] == code)
        .unwrap_or_else(|| panic!("missing {code}: {messages:?}"))
}

fn files(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn compiler_binary_checks_real_new_source_fixtures_without_artifacts() {
    for text in [
        VALID,
        include_str!("../../../crates/typecheck/examples/window-source.gin"),
        "Marker has
finish(value Marker):
    Marker := value
    return
drain(values (Marker, Marker)):
    for value in values do
        finish(value)
    return
",
    ] {
        let (directory, path) = fixture(text);
        let before = files(directory.path());
        let output = command(&path)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let messages = messages(&output);
        assert_eq!(finished(&messages)["success"], true);
        assert_eq!(finished(&messages)["errors"], 0);
        assert_eq!(files(directory.path()), before);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Built"));
    }
}

#[test]
fn semantic_failure_points_at_the_failing_expression_inside_its_construct() {
    let text = format!("-- hé 🦀\ngood(): return\n{INVALID}");
    let (directory, path) = fixture(&text);
    let source = parser::parse_strict(&text).unwrap();
    let construct = source.span_table.get(source.items[1].span_id);
    let output = command(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let messages = messages(&output);
    let failure = diagnostic(&messages, "source-check-unproved");
    assert_eq!(
        failure["file"],
        path.normalize_diagnostic_path().to_str().unwrap()
    );
    // The primary range is the exact expression the checker was proving (the returned `0`),
    // which lies inside the construct; the construct itself is the related range. This is the
    // same data the REPL and the JSONL protocol report (`crate::flaw`).
    let start = usize::try_from(failure["range"]["byte_start"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(failure["range"]["byte_end"].as_u64().unwrap()).unwrap();
    assert_eq!(&text[start..end], "0");
    assert!(construct.start() <= start && end <= construct.end());
    assert_eq!(failure["label"], "the expression being checked");
    let related = &failure["related"][0];
    assert_eq!(related["label"], "enclosing source construct");
    assert_eq!(related["range"]["byte_start"], construct.start());
    assert_eq!(related["range"]["byte_end"], construct.end());
    assert_eq!(failure["range"]["start"]["line"], 4);
    // What the checker had proved there is reported as arguments.
    let facts: Vec<&Value> = failure["args"].as_array().unwrap().iter().collect();
    assert!(
        facts
            .iter()
            .any(|a| a[1] == "0 <= value <= 16" || a.to_string().contains("0 <= value <= 16")),
        "{facts:?}"
    );
    assert_eq!(failure["dependency"], false);
    assert_eq!(finished(&messages)["success"], false);
    assert_eq!(finished(&messages)["errors"], 1);
}

#[test]
fn parser_failures_keep_parser_codes_and_never_reach_the_checker() {
    let text = "-- hé 🦀\nbroken := @\n";
    let (_directory, path) = fixture(text);
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let messages = messages(&output);
    let failures: Vec<_> = messages
        .iter()
        .filter(|message| message["reason"] == "diagnostic")
        .collect();
    assert!(!failures.is_empty());
    assert!(failures.iter().all(|message| {
        message["code"].as_str().unwrap().starts_with("parse-")
            || message["code"].as_str().unwrap().starts_with("lex-")
    }));
    let at = text.find('@').unwrap();
    assert!(failures.iter().any(|message| {
        message["range"]["byte_start"]
            .as_u64()
            .is_some_and(|start| start >= at as u64)
    }));
    assert_eq!(finished(&messages)["success"], false);
}

#[test]
fn unsupported_source_is_not_silently_checked_by_another_engine() {
    // Division used to be the construct the new checker lacked; it now has `/` and the other
    // integer operations (`integer_ops.rs`), so a comparison stands in for "legacy-valid, not
    // yet supported here".
    let text = "main: 1 = 2\n";
    let (_directory, path) = fixture(text);
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let messages = messages(&output);
    diagnostic(&messages, "source-check-unsupported");
    assert_eq!(finished(&messages)["success"], false);
}

#[test]
fn loose_file_check_ignores_unrelated_sources_and_writes_requested_profile() {
    let (directory, path) = fixture(VALID);
    std::fs::write(directory.path().join("other.gin"), "not valid Gin @@@").unwrap();
    let profile = directory.path().join("profile.json");
    std::fs::write(&profile, "leave unchanged").unwrap();
    let before = files(directory.path());
    let output = command(&path)
        .current_dir(directory.path())
        .env("GIN_PROFILE_JSON", &profile)
        .env("GINC_TIMINGS", "1")
        .env("GINC_FAST_LLVM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(finished(&messages(&output))["success"], true);
    let profile: Value = serde_json::from_slice(&std::fs::read(profile).unwrap()).unwrap();
    assert!(profile.is_object(), "{profile:?}");
    assert_eq!(files(directory.path()), before);
}

#[test]
fn all_omitted_errors_still_mean_failure() {
    let (_directory, path) = fixture(INVALID);
    let output = command(&path).arg("--max-errors=0").output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let messages = messages(&output);
    assert!(
        messages
            .iter()
            .all(|message| message["reason"] != "diagnostic")
    );
    let summary = finished(&messages);
    assert_eq!(summary["success"], false);
    assert_eq!(summary["errors"], 1);
    assert_eq!(summary["omitted_errors"], 1);
}

#[test]
fn check_rejects_native_options_even_when_explicitly_defaulted() {
    let (directory, path) = fixture(VALID);
    let options: &[&[&str]] = &[
        &["--profile", "release"],
        &["--output", "do-not-create"],
        &["--no-cache"],
        &["--fast-llvm"],
    ];
    for options in options {
        let output = command(&path)
            .current_dir(directory.path())
            .args(*options)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{options:?}: {output:?}");
        assert_eq!(files(directory.path()), ["input.gin"]);
    }
}

#[test]
fn reporting_options_work_with_the_default_check_action() {
    let (_directory, path) = fixture(VALID);
    for options in [
        &["--message-format", "json"][..],
        &["--max-errors", "1"][..],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
            .arg(&path)
            .args(options)
            .env_remove("GIN_PROFILE_JSON")
            .output()
            .unwrap();
        assert!(output.status.success(), "{options:?}: {output:?}");
    }
}

#[test]
fn a_directory_without_a_manifest_is_not_a_package_and_is_not_discovered() {
    let (directory, _path) = fixture(VALID);
    let before = files(directory.path());
    let output = command(directory.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let messages = messages(&output);
    assert_eq!(finished(&messages)["success"], false);
    assert_eq!(files(directory.path()), before);
}

#[test]
fn programmatic_entry_points_check_without_artifacts() {
    let (directory, path) = fixture(VALID);
    for entry in 0..3 {
        let mut args = Args {
            input: path.clone(),
            emit: Emit::Check,
            message_format: MessageFormat::Json,
            ..Default::default()
        };
        let result = match entry {
            0 => GinCompiler::compile(&mut args),
            1 => GinCompiler::compile_cli(&mut args),
            2 => GinCompiler::check(&mut args, CheckOptions::default()),
            _ => unreachable!(),
        };
        assert!(matches!(result, CompileResult::Success { output: None }));
        assert_eq!(files(directory.path()), ["input.gin"]);

        args.emit = Emit::Obj;
        let result = GinCompiler::check(&mut args, CheckOptions::default());
        assert!(matches!(result, CompileResult::Success { output: None }));
        assert_eq!(files(directory.path()), ["input.gin"]);
    }
}

const MANIFEST: &str = "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n";

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

#[test]
fn a_package_directory_checks_across_its_folder_modules() {
    let directory = package(&[
        (
            "main.gin",
            "use app.util five\nagain() in 0 to 16: return five()\n",
        ),
        ("util/five.gin", "five() in 0 to 16: return 5\n"),
    ]);
    let before = files(directory.path());
    let output = command(directory.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(finished(&messages(&output))["success"], true);
    assert_eq!(files(directory.path()), before);
}

#[test]
fn a_package_failure_is_located_in_the_file_that_caused_it() {
    let directory = package(&[
        ("main.gin", "use app.util missing\n"),
        ("util/five.gin", "five() in 0 to 16: return 5\n"),
    ]);
    let output = command(directory.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let messages = messages(&output);
    let found = diagnostic(&messages, "source-check-unknown-name");
    assert!(found.to_string().contains("main.gin"), "{found}");
    assert_eq!(finished(&messages)["success"], false);
}

#[test]
fn package_syntax_errors_name_their_file_and_cycles_are_reported() {
    let broken = package(&[("main.gin", "bad(\n")]);
    let output = command(broken.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("main.gin"),
        "{output:?}"
    );
    let cyclic = package(&[
        ("main.gin", "use app.a one\n"),
        ("a/a.gin", "use app.a one\n"),
    ]);
    let output = command(cyclic.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("import cycle"),
        "{output:?}"
    );
}

const REDUNDANT: &str = "#unrestricted\nByte is in 0 to 255\nByte has\n    finish(self): return\n";

#[test]
fn a_redundant_terminal_is_a_located_warning_that_does_not_fail_the_check() {
    let (directory, path) = fixture(REDUNDANT);
    let output = command(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let messages = messages(&output);
    let warning = diagnostic(&messages, "source-check-redundant-terminal");
    assert_eq!(warning["severity"], "warn", "{warning}");
    assert!(warning["message"].as_str().unwrap().contains("`finish`"));
    let span = &warning["range"];
    let start = usize::try_from(span["byte_start"].as_u64().unwrap()).unwrap();
    assert_eq!(&REDUNDANT[start..start + 4], "fini");
    let summary = finished(&messages);
    assert_eq!(summary["success"], true);
    assert_eq!(summary["warnings"], 1);
    assert_eq!(summary["errors"], 0);
}

#[test]
fn a_redundant_terminal_in_a_package_module_names_its_file() {
    let directory = package(&[
        ("main.gin", "use app.util Byte\n"),
        ("util/byte.gin", REDUNDANT),
    ]);
    let output = command(directory.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let messages = messages(&output);
    let warning = diagnostic(&messages, "source-check-redundant-terminal");
    assert!(warning.to_string().contains("byte.gin"), "{warning}");
    assert_eq!(finished(&messages)["warnings"], 1);
}

#[test]
fn a_failure_message_names_the_culprit_when_the_checker_knows_it() {
    let text = "#unrestricted\nBox(element) has item element\nRes has v in 0 to 9\nuse_it(b Box(Res)): return\n";
    let (directory, path) = fixture(text);
    let output = command(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let messages = messages(&output);
    let failure = diagnostic(&messages, "source-check-unrestricted-argument");
    let message = failure["message"].as_str().unwrap();
    assert!(
        message.contains("`Res`") && message.contains("`Box`"),
        "{message}"
    );
}

#[test]
fn unreachable_code_is_a_located_warning_that_does_not_fail_the_check() {
    let text = "f(x in 0 to 9) in 0 to 9:\n  return when x < 0 then 1 else x\n";
    let (directory, path) = fixture(text);
    let output = command(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let messages = messages(&output);
    let warning = diagnostic(&messages, "source-check-unreachable-code");
    assert_eq!(warning["severity"], "warn", "{warning}");
    let start = usize::try_from(warning["range"]["byte_start"].as_u64().unwrap()).unwrap();
    assert_eq!(&text[start..start + 1], "1");
    assert_eq!(finished(&messages)["warnings"], 1);
}

#[test]
fn an_unchecked_cast_is_located_at_the_operation_and_accepted_by_the_marker() {
    let prelude = "#unrestricted\nByte is in 0 to 255\n#unrestricted\nAddress is in 0 to 65535\n";
    let missing = format!("{prelude}raw(address Address) @Byte: return address as @Byte\n");
    let (_directory, path) = fixture(&missing);
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let reported = messages(&output);
    let failure = diagnostic(&reported, "source-check-missing-acceptance");
    let start = usize::try_from(failure["range"]["byte_start"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(failure["range"]["byte_end"].as_u64().unwrap()).unwrap();
    assert_eq!(&missing[start..end], "address as @Byte");
    assert!(
        failure["message"].as_str().unwrap().contains("#unchecked("),
        "{failure}"
    );

    let accepted =
        format!("{prelude}raw(address Address) @Byte: return #unchecked(address as @Byte)\n");
    let (_directory, path) = fixture(&accepted);
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    // A marker on a checked cast is its own error.
    let stale =
        format!("{prelude}narrow(address in 0 to 100) Byte: return #unchecked(address as Byte)\n");
    let (_directory, path) = fixture(&stale);
    let output = command(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    diagnostic(&messages(&output), "source-check-unneeded-acceptance");
}

/// A package over the draft `modules/core` library, linked in as `./core`.
fn core_package(main: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"probe\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), main).unwrap();
    directory
}

const LIST_CLIENT: &str = include_str!("fixtures/list_client.gin");

fn check_core(main: &str) -> Vec<Value> {
    let directory = core_package(main);
    let output = command(directory.path()).output().unwrap();
    messages(&output)
}

#[test]
fn the_core_list_over_spans_checks_for_a_client() {
    let reported = check_core(LIST_CLIENT);
    assert_eq!(finished(&reported)["success"], true, "{reported:?}");
}

#[test]
fn the_core_list_refuses_a_push_past_capacity_and_a_leaked_list() {
    let third = LIST_CLIENT.replace(
        "        items.push(Token(id: 2))\n",
        "        items.push(Token(id: 2))\n        items.push(Token(id: 3))\n",
    );
    let reported = check_core(&third);
    diagnostic(&reported, "source-check-unproved");
    let leaked = LIST_CLIENT.replace(
        "        items.release\n        return\n    Failed(failure)",
        "        return\n    Failed(failure)",
    );
    assert_ne!(leaked, LIST_CLIENT);
    let reported = check_core(&leaked);
    diagnostic(&reported, "source-check-unconsumed");
}

const CELL: &str = "use core.span Extent, Vacant, unmap\n\nCell has\n    n in 0 to 9\n    end(self):\n        Cell(n) := self\n        return\n\n";

#[test]
fn an_extent_is_claimed_once_and_every_page_goes_back() {
    let ok = format!(
        "{CELL}run(count in 0 to 4):\n    m: Extent.map(4096)\n    when m is Mapped(token) then\n        claimed: token.claim(Cell, 1)\n        unmap(Cell, claimed)\n        return\n    return\n\n#entry\nmain: return\n"
    );
    let reported = check_core(&ok);
    assert_eq!(finished(&reported)["success"], true, "{reported:?}");
}

#[test]
fn the_boundary_token_refuses_double_use_forgery_dropping_and_stale_joins() {
    let entry = "\n#entry\nmain: return\n";
    // Claiming the same token twice.
    let twice = format!(
        "{CELL}run():\n    m: Extent.map(4096)\n    when m is Mapped(token) then\n        first: token.claim(Cell, 1)\n        second: token.claim(Cell, 1)\n        return\n    return\n{entry}"
    );
    diagnostic(&check_core(&twice), "source-check-use-after-move");
    // Making a token (or a span) from a number.
    let forged =
        format!("{CELL}forge(item Type) Extent: return Extent(base: 4096, size: 4096)\n{entry}");
    diagnostic(&check_core(&forged), "source-check-privacy");
    let span = format!(
        "{CELL}forge(item Type) Vacant(item): return Vacant(base: 4096, first: 0, count: 1)\n{entry}"
    );
    diagnostic(&check_core(&span), "source-check-privacy");
    // Dropping a token.
    let dropped = format!(
        "{CELL}run():\n    m: Extent.map(4096)\n    when m is Mapped(token) then\n        return\n    return\n{entry}"
    );
    diagnostic(&check_core(&dropped), "source-check-unconsumed");
    // Claiming more slots than the token's bytes cover is a claim the caller cannot prove.
    let too_many = format!(
        "{CELL}run(count in 0 to 100):\n    m: Extent.map(8)\n    when m is Mapped(token) then\n        claimed: token.claim(Cell, count)\n        unmap(Cell, claimed)\n        return\n    return\n{entry}"
    );
    diagnostic(&check_core(&too_many), "source-check-unproved");
    // A span of an earlier extent cannot be joined to a span of another.
    let stale = format!(
        "{CELL}join(item Type, old mut Vacant(item), fresh Vacant(item)):\n    old.absorb_front(fresh)\n    return\n{entry}"
    );
    diagnostic(&check_core(&stale), "source-check-unproved");
}

/// The functions of `module` (by name) whose bodies the recorder placed.
fn recorded_in(main: &str, module: &str) -> Vec<(String, bool)> {
    let directory = core_package(main);
    let package = package::Package::open(directory.path()).unwrap();
    let work = typecheck::Limits {
        work_max: typecheck::Limits::default().work_max * 160,
        ..typecheck::Limits::default()
    };
    let checked = package
        .check(package::GraphLimits::default(), work)
        .unwrap_or_else(|_| panic!("the package checks"));
    let capsule = checked
        .capsules
        .iter()
        .find(|capsule| capsule.name() == module)
        .expect("module present");
    let file = capsule.checked();
    file.functions
        .iter()
        .zip(&file.bodies)
        .map(|(function, body)| (function.name.clone(), body.is_some()))
        .collect()
}

#[test]
fn the_system_wrappers_and_the_kernel_calls_have_recorded_bodies() {
    let main = "use core.target.kernel send, receive\nuse core.alloc.system reserve_bytes, release_bytes, write_bytes\n#entry\nmain: return\n";
    let system = recorded_in(main, "core.alloc.system");
    for name in [
        "reserve_address",
        "reserve_bytes",
        "release_bytes",
        "write_bytes",
    ] {
        let found = system
            .iter()
            .find(|(function, _)| function == name)
            .unwrap_or_else(|| panic!("{name}"));
        assert!(found.1, "{name} has no recorded body: {system:?}");
    }
    let kernel = recorded_in(main, "core.target.kernel");
    for name in ["send", "receive"] {
        let found = kernel
            .iter()
            .find(|(function, _)| function == name)
            .unwrap_or_else(|| panic!("{name}"));
        assert!(found.1, "{name} has no recorded body: {kernel:?}");
    }
}

// The trusted view of a live span (`Live.view`, `Live.view_mut`): ordinary checked Gin around one
// `#unchecked(...)` cast. The view borrows the span.
const VIEWED: &str = "use core.span Live\nuse core.slice Byte, Length, SliceView, SliceMutView\n\n";

#[test]
fn a_live_span_lends_a_view_whose_elements_are_borrowed_with_the_bound_proved() {
    let ok = format!(
        "{VIEWED}first(span ref Live(Byte) and span.start = 0 and span.length > 0) Byte:\n    bytes: span.view\n    b: bytes.borrow(0)\n    return deref b\n\n#entry\nmain: return\n"
    );
    let reported = check_core(&ok);
    assert_eq!(finished(&reported)["success"], true, "{reported:?}");
    // The span must start its block (the view is the array at the block's address).
    let offset = ok.replace(" and span.start = 0", "");
    diagnostic(&check_core(&offset), "source-check-unproved");
    // Index past the length.
    let past = ok.replace(" and span.length > 0", "");
    diagnostic(&check_core(&past), "source-check-unproved");
}

#[test]
fn a_view_blocks_what_changes_or_ends_its_span_and_a_vacant_span_has_none() {
    let entry = "\n#entry\nmain: return\n";
    // A second exclusive view while the first lives.
    let both = format!(
        "{VIEWED}twice(span mut Live(Byte) and span.start = 0 and span.length > 0):\n    shared: span.view\n    unique: span.view_mut\n    b: shared.borrow(0)\n    return\n{entry}"
    );
    diagnostic(&check_core(&both), "source-check-borrow-conflict");
    // Ending the span (finish) while a view lives.
    let ended = format!(
        "{VIEWED}ends(span Live(Byte) and span.start = 0 and span.length = 0):\n    shared: span.view\n    span.finish\n    n: shared.length\n    return\n{entry}"
    );
    assert_ne!(finished(&check_core(&ended))["success"], true);
    // No view of a vacant span: it holds no values.
    let vacant = "use core.span Vacant\nuse core.slice Byte\n\nlook(span ref Vacant(Byte)):\n    bytes: span.view\n    return\n\n#entry\nmain: return\n";
    diagnostic(&check_core(vacant), "source-check-unknown-name");
}

#[test]
#[ignore]
fn print_unrecorded_core_bodies() {
    let main = "use core.span Live\nuse core.vector Vector\nuse core.collection List\nuse core.string String\nuse core.io write\n#entry\nmain: return\n";
    let directory = core_package(main);
    let given = std::env::var("GIN_DUMP_PACKAGE").ok();
    let path = given
        .as_deref()
        .map(std::path::Path::new)
        .unwrap_or(directory.path());
    let package = package::Package::open(path).unwrap();
    let work = typecheck::Limits {
        work_max: typecheck::Limits::default().work_max * 160,
        ..typecheck::Limits::default()
    };
    let checked = package
        .check(package::GraphLimits::default(), work)
        .unwrap_or_else(|_| panic!("the package checks"));
    for capsule in &checked.capsules {
        let file = capsule.checked();
        for (index, (function, body)) in file.functions.iter().zip(&file.bodies).enumerate() {
            if let Some(body) = body
                && function.name.ends_with("as_num")
            {
                println!(
                    "BODY {}::{} effects={} kinds={:?}",
                    capsule.name(),
                    function.name,
                    body.effects,
                    body.expressions
                        .iter()
                        .map(|e| format!("{:?}", e.kind))
                        .collect::<Vec<_>>()
                );
            }
            if body.is_none() && function.instruction.is_none() && function.memory.is_none() {
                let why = file
                    .declines
                    .iter()
                    .find(|(at, _)| *at as usize == index)
                    .map(|(_, d)| {
                        format!(
                            "{} @{:?}",
                            d.construct,
                            d.span.map(|s| (s.start(), s.end()))
                        )
                    })
                    .unwrap_or_default();
                println!(
                    "UNRECORDED {}::{} -- {}",
                    capsule.name(),
                    function.name,
                    why
                );
            }
        }
    }
}

#[test]
fn a_client_of_the_io_and_string_modules_together_is_within_the_definition_limit() {
    // Each import brings its dependency's whole function table: together these exceeded the 4096
    // a single file is given (`definitions_max`), reported as a work budget.
    let main = "use core.io write\nuse core.string String\nuse core.slice Byte\n\nshow(text ref String(GlobalAllocator)):\n    write(text.as_bytes)\n    return\n\n#entry\nmain: return\n";
    let both = format!("use core.alloc GlobalAllocator\n{main}");
    let reported = check_core(&both);
    assert_eq!(finished(&reported)["success"], true, "{reported:?}");
}

const PROVIDER: &str = "\
use core.ops AllocError, Release
use core.span Vacant, Extent, max_slots, max_extent
use core.alloc Allocator

#unrestricted
Mine has Allocator and Release
    reserve(self mut, x Type, count in 0 to max_slots) Reserved(storage Vacant(x)) where (storage.length = count and storage.start = 0) or
      Failed(AllocError):
        alignment: #alignment(x)
        when alignment > 4096 then
            return Failed(AllocError.AddressSpaceLimitExceeded)
        width: #size(x)
        requested: count * width
        when requested > max_extent then
            return Failed(AllocError.AddressSpaceLimitExceeded)
        mapped: Extent.map(requested)
        when mapped is Mapped(token) then
            claimed: token.claim(x, count)
            return Reserved(claimed)
        return Failed(AllocError.OutOfMemory)
    reclaim(self mut, x Type, storage Vacant(x)):
        storage.release
        return
    release(self):
        return
#entry
main: return
";

/// A trait provider whose `reserve` returns a generic member from one path and an error union
/// member from the others: the choices between them need the function's own result sum.
#[test]
fn a_provider_whose_paths_return_different_members_of_its_result_is_recorded() {
    let reserve = |main: &str| {
        recorded_in(main, "probe")
            .into_iter()
            .find(|(name, _)| name == "Mine.reserve")
            .map(|(_, recorded)| recorded)
    };
    assert_eq!(reserve(PROVIDER), Some(true));
    // Negative control: without the guard on the size, the map call and the claim are refused.
    let unguarded = PROVIDER.replace("        when requested > max_extent then\n            return Failed(AllocError.AddressSpaceLimitExceeded)\n", "");
    diagnostic(&check_core(&unguarded), "source-check-type");
}

const GROW: &str = "\
use core.vector Vector
use core.slice Byte
use core.span max_slots
use core.ops Release

grow(l mut Vector(Byte), start in 0 to max_slots and start = l.length, count in 0 to max_slots and start + count <= l.capacity) where l.length :: start + count:
    for index in 0 to count - 1 where l.length = start + index and start + count <= l.capacity do
        l.push(104 as Byte)
    return

#entry
main: return
";

/// The loop invariant over the library's own `Vector`: its `length` is a plain field read, so it
/// is one number before and after a push, and the loop's total effect is known without a `::`.
#[test]
fn a_loop_invariant_over_a_core_vector_gives_the_total_length() {
    let reported = check_core(GROW);
    assert_eq!(finished(&reported)["success"], true, "{reported:?}");
    // Negative controls: no invariant (the push needs a moving bound), and a wrong total.
    let bare = GROW.replace(
        " where l.length = start + index and start + count <= l.capacity",
        "",
    );
    diagnostic(&check_core(&bare), "source-check-unproved");
    let wrong = GROW.replace(
        "where l.length :: start + count:",
        "where l.length :: start + count + 1:",
    );
    diagnostic(&check_core(&wrong), "source-check-unproved");
}

/// `MemorySize.add`/`subtract` (and the checked arithmetic of `SignedBigInt`) return a number or
/// a payload member of a sum from the two sides of a `when .. is in ..`: all recorded.
#[test]
fn the_checked_arithmetic_of_the_system_and_primitive_domains_is_recorded() {
    let main =
        "use core.alloc.system MemorySize\nuse core.primitive SignedBigInt\n#entry\nmain: return\n";
    let system = recorded_in(main, "core.alloc.system");
    for name in ["MemorySize.add", "MemorySize.subtract"] {
        let found = system
            .iter()
            .find(|(function, _)| function == name)
            .unwrap_or_else(|| panic!("{name}"));
        assert!(found.1, "{name} has no recorded body: {system:?}");
    }
    let primitive = recorded_in(main, "core.primitive");
    for name in [
        "SignedBigInt.add",
        "SignedBigInt.subtract",
        "SignedBigInt.multiply",
    ] {
        let found = primitive
            .iter()
            .find(|(function, _)| function == name)
            .unwrap_or_else(|| panic!("{name}"));
        assert!(found.1, "{name} has no recorded body: {primitive:?}");
    }
    // Negative control: a function that still cannot be recorded stays unrecorded (`shift_right`
    // returns through a form the recorder does not place; `divide` and `modulo` became recordable
    // with negative literals and the guard narrowing).
    let shift = primitive
        .iter()
        .find(|(function, _)| function == "SignedBigInt.shift_right")
        .unwrap();
    assert!(!shift.1, "{primitive:?}");
}

const TRANSITIONS: &str = "\
use core.slice Byte
use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.ops Release

fill(buffer mut List(Byte, GlobalAllocator) and buffer.length = 0 and buffer.capacity >= 19 and buffer.capacity <= 4096) where buffer.length :: 19:
    for index in 0 to 18 where buffer.length = index and buffer.capacity >= 19 and buffer.capacity <= 4096 do
        buffer.push(7 as Byte)
    return

use_it:
    attempt: List.with_capacity(Byte, GlobalAllocator, 19, GlobalAllocator())
    when attempt is Allocated(items) then
        fill(items)
        n in 19 to 19: items.length
        items.release
        return
    Failed(failure) := attempt
    failure.release
    return

write_through(b mut List(Byte, GlobalAllocator) and b.length = 19):
    w: b.view_mut
    w.data.[2] :: 48 as Byte
    return

#entry
main: return
";

/// A transition to a constant (`length :: 19`) is applied at the call, a bound on a field a loop
/// call does not name survives the call (`capacity >= 19` as an invariant), and an element is
/// written through a mutable view held in a local.
#[test]
fn constant_transitions_kept_bounds_and_writes_through_a_local_view_check() {
    let reported = check_core(TRANSITIONS);
    assert_eq!(finished(&reported)["success"], true, "{reported:?}");
    // Negative controls: the constant is the transition's, not another number.
    let wrong = TRANSITIONS.replace("n in 19 to 19: items.length", "n in 20 to 20: items.length");
    diagnostic(&check_core(&wrong), "source-check-type");
    // The bound is needed: without it the push has no room. (The loop keeps the capacity the call
    // leaves unchanged, so the invariant need not repeat the bound, but the entry must give it.)
    let bare = TRANSITIONS.replace(
        " and buffer.length = 0 and buffer.capacity >= 19 and buffer.capacity <= 4096)",
        " and buffer.length = 0)",
    );
    diagnostic(&check_core(&bare), "source-check-unproved");
    // The write stays inside the view.
    let past = TRANSITIONS.replace("w.data.[2]", "w.data.[19]");
    diagnostic(&check_core(&past), "source-check-unproved");
}

const FRAME: &str = "\
use core.slice Byte
use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.ops Release

fill(buffer mut List(Byte, GlobalAllocator) and buffer.length = 0 and buffer.capacity = 19) where buffer.length :: 19, buffer.capacity :: buffer.capacity:
    for index in 0 to 18 where buffer.length = index do
        buffer.push(7 as Byte)
    return

#entry
main: return
";

/// A loop keeps what a call leaves as it found it: `push` promises `capacity :: capacity`, so the
/// capacity at the head of every pass, and after the loop, is the one the loop was entered with, with
/// no invariant line for it. Controls: a call that says nothing, a different receiver, and what the
/// body writes (the length) each lose it.
#[test]
fn a_loop_keeps_the_fields_a_call_leaves_unchanged() {
    let reported = check_core(FRAME);
    assert_eq!(finished(&reported)["success"], true, "{reported:?}");
    // A call whose contract is silent may change the capacity.
    let silent = FRAME.replace(
        "        buffer.push(7 as Byte)\n",
        "        touch(buffer)\n",
    ).replace(
        "#entry\nmain: return\n",
        "touch(buffer mut List(Byte, GlobalAllocator) and buffer.length < buffer.capacity):\n    buffer.push(7 as Byte)\n    return\n\n#entry\nmain: return\n",
    );
    diagnostic(&check_core(&silent), "source-check-unproved");
    // The fact is about the receiver the loop was entered with, not another list.
    let other = FRAME
        .replace("fill(buffer mut List(Byte, GlobalAllocator) and", "fill(other mut List(Byte, GlobalAllocator), buffer mut List(Byte, GlobalAllocator) and")
        .replace("buffer.capacity :: buffer.capacity:", "buffer.capacity :: other.capacity:");
    diagnostic(&check_core(&other), "source-check-unproved");
    // What the body writes is forgotten at the head: with no invariant on the length the total is
    // not known.
    let forgotten = FRAME
        .replace(
            "buffer.capacity = 19) where",
            "buffer.capacity = 19, n in 0 to 18) where",
        )
        .replace(
            "for index in 0 to 18 where buffer.length = index do",
            "for index in 0 to n do",
        );
    diagnostic(&check_core(&forgotten), "source-check-unproved");
}

const FRAME_WHILE: &str = "\
use core.slice Byte
use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.ops Release

fill(buffer mut List(Byte, GlobalAllocator) and buffer.length = 0 and buffer.capacity = 19) where buffer.capacity :: buffer.capacity:
    while buffer.length < 19 do
        buffer.push(7 as Byte)
    return

#entry
main: return
";

/// The same for a `while` loop: the capacity `push` leaves unchanged is the entry's at every head
/// (so `length < 19` gives the push its room) and after the loop. Controls as for a range loop.
#[test]
fn a_while_loop_keeps_the_fields_a_call_leaves_unchanged() {
    let reported = check_core(FRAME_WHILE);
    assert_eq!(finished(&reported)["success"], true, "{reported:?}");
    let silent = FRAME_WHILE
        .replace("        buffer.push(7 as Byte)\n", "        touch(buffer)\n")
        .replace(
            "#entry\nmain: return\n",
            "touch(buffer mut List(Byte, GlobalAllocator) and buffer.length < buffer.capacity):\n    buffer.push(7 as Byte)\n    return\n\n#entry\nmain: return\n",
        );
    diagnostic(&check_core(&silent), "source-check-unproved");
    let other = FRAME_WHILE
        .replace("fill(buffer mut List(Byte, GlobalAllocator) and", "fill(other mut List(Byte, GlobalAllocator), buffer mut List(Byte, GlobalAllocator) and")
        .replace("buffer.capacity :: buffer.capacity:", "buffer.capacity :: other.capacity:");
    diagnostic(&check_core(&other), "source-check-unproved");
    // The length the body writes is forgotten: after the loop it is not known to be 19.
    let length = FRAME_WHILE.replace(
        "where buffer.capacity :: buffer.capacity:",
        "where buffer.capacity :: buffer.capacity, buffer.length :: 19:",
    );
    diagnostic(&check_core(&length), "source-check-unproved");
}

// Dependency warnings (decision D6): hidden by default, shown by `--warnings deps`.

fn repository(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn check_hello(extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "check", "--message-format=json"])
        .args(extra)
        .arg(repository("modules/popular_problems/hello_world"))
        .env_remove("GIN_PROFILE_JSON")
        .output()
        .unwrap()
}

#[test]
fn warnings_of_a_dependency_are_hidden_by_default_and_counted() {
    let output = check_hello(&[]);
    assert!(output.status.success(), "{output:?}");
    let messages = messages(&output);
    assert!(
        messages
            .iter()
            .all(|message| message["reason"] != "diagnostic"),
        "{messages:?}"
    );
    let summary = finished(&messages);
    assert_eq!(summary["warnings"], 0);
    assert!(
        summary["hidden_warnings"].as_u64().unwrap() > 0,
        "{summary}"
    );
    let human = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "check"])
        .arg(repository("modules/popular_problems/hello_world"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&human.stderr);
    assert!(
        text.contains("dependency warnings hidden; --warnings deps shows them"),
        "{text}"
    );
    assert!(!text.contains("redundant-terminal"), "{text}");
}

#[test]
fn warnings_deps_shows_them_marked_as_from_a_dependency() {
    let hidden = finished(&messages(&check_hello(&[]))).clone();
    let output = check_hello(&["--warnings", "deps"]);
    assert!(output.status.success(), "{output:?}");
    let messages = messages(&output);
    let shown: Vec<_> = messages
        .iter()
        .filter(|m| m["reason"] == "diagnostic")
        .collect();
    assert_eq!(
        shown.len() as u64,
        hidden["hidden_warnings"].as_u64().unwrap()
    );
    assert!(
        shown
            .iter()
            .all(|m| m["severity"] == "warn" && m["dependency"] == true),
        "{shown:?}"
    );
    assert!(
        shown
            .iter()
            .all(|m| m["file"].as_str().unwrap().contains("modules/core"))
    );
    let summary = finished(&messages);
    assert_eq!(summary["warnings"], hidden["hidden_warnings"]);
    assert_eq!(summary["hidden_warnings"], 0);
}

#[test]
fn warnings_of_the_package_itself_are_always_shown() {
    let (_directory, path) =
        fixture("f(a in 0 to 9) in 0 to 9:\n    when a < 5 then return 1\n    return #todo\n");
    let output = command(&path).output().unwrap();
    let messages = messages(&output);
    assert_eq!(finished(&messages)["warnings"], 1, "{messages:?}");
    assert_eq!(finished(&messages)["hidden_warnings"], 0);
    assert!(
        messages
            .iter()
            .any(|m| m["reason"] == "diagnostic" && m["dependency"] == false)
    );
}

#[test]
fn errors_of_a_dependency_are_never_hidden() {
    let workspace = tempfile::tempdir().unwrap();
    let lib = workspace.path().join("lib");
    let app = workspace.path().join("app");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        lib.join("mod.flask"),
        "#settings\nmodule: \"lib\"\nversion: 0.1.0\ntarget: \"library\"\n",
    )
    .unwrap();
    std::fs::write(lib.join("util.gin"), "bad() in 0 to 1: return 5\n").unwrap();
    std::fs::write(app.join("mod.flask"), "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n\n#dependencies\nlib: \"../lib\"\n").unwrap();
    std::fs::write(
        app.join("main.gin"),
        "use lib bad\nagain() in 0 to 1: return 1\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "check", "--message-format=json"])
        .arg(&app)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let messages = messages(&output);
    assert_eq!(
        diagnostic(&messages, "source-check-type")["severity"],
        "flaw"
    );
    assert_eq!(
        diagnostic(&messages, "source-check-type")["dependency"],
        true
    );
    std::fs::write(lib.join("util.gin"), "bad(\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "check", "--message-format=json"])
        .arg(&app)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let records = self::messages(&output);
    let parser_failure = records
        .iter()
        .find(|record| record["reason"] == "diagnostic")
        .unwrap();
    assert!(
        parser_failure["code"]
            .as_str()
            .unwrap()
            .starts_with("parse-")
    );
    assert_eq!(parser_failure["dependency"], true);
}
