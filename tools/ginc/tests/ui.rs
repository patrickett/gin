use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::Duration,
};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ui");
const MAX_FIXTURES: usize = 256;
const PROCESS_OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Directives {
    mode: Option<String>,
    target: Option<String>,
    revisions: BTreeSet<String>,
    stdout: Option<String>,
    exit_code: Option<i32>,
    snapshot_mlir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Diagnostic {
    severity: String,
    code: String,
    span: String,
    message: String,
}

// TODO(ui-packages): model a manifest and support files as one case, and retain fixture-relative
// diagnostic filenames, before moving cross-module/import tests out of Rust.
struct UiCase {
    fixture: PathBuf,
    source: String,
    directives: Directives,
    snapshot: PathBuf,
}

fn cases_for(root: &Path, fixture: &Path) -> Vec<UiCase> {
    let source = fs::read_to_string(fixture).unwrap();
    let directives =
        parse_directives(&source).unwrap_or_else(|error| panic!("{}: {error}", fixture.display()));
    validate_revision_gates(&source, &directives)
        .unwrap_or_else(|error| panic!("{}: {error}", fixture.display()));
    if fixture
        .components()
        .any(|part| part.as_os_str() == "crashes")
    {
        assert_eq!(directives.mode.as_deref(), Some("compile-fail"));
    }
    let stem = fixture.strip_prefix(root).unwrap().with_extension("");
    let revisions: Vec<_> = if directives.revisions.is_empty() {
        vec![None]
    } else {
        directives.revisions.iter().map(Some).collect()
    };
    revisions
        .into_iter()
        .map(|revision| {
            let (source, snapshot) = match revision {
                Some(revision) => (
                    revision_source(&source, revision),
                    stem.with_file_name(format!(
                        "{}.{revision}.diag",
                        stem.file_name().unwrap().to_string_lossy()
                    )),
                ),
                None => (source.clone(), stem.with_extension("diag")),
            };
            UiCase {
                fixture: fixture.to_owned(),
                source,
                directives: directives.clone(),
                snapshot,
            }
        })
        .collect()
}

fn case_snapshots(case: &UiCase) -> Vec<PathBuf> {
    let mut paths = vec![case.snapshot.clone()];
    if case.directives.snapshot_mlir {
        paths.push(case.snapshot.with_extension("body"));
        paths.push(case.snapshot.with_extension("mlir"));
    }
    paths
}

fn expected_snapshots(cases: &[UiCase]) -> Result<BTreeSet<PathBuf>, String> {
    let mut owners = BTreeMap::new();
    for case in cases {
        for snapshot in case_snapshots(case) {
            if let Some(previous) = owners.insert(snapshot.clone(), case.fixture.clone()) {
                return Err(format!(
                    "snapshot {} is owned by both {} and {}",
                    snapshot.display(),
                    previous.display(),
                    case.fixture.display()
                ));
            }
        }
    }
    Ok(owners.into_keys().collect())
}

#[test]
fn ui_suite() {
    let root = Path::new(ROOT);
    let fixtures = discover(root).unwrap_or_else(|error| panic!("UI discovery: {error}"));
    let bless = std::env::var_os("GIN_BLESS").as_deref() == Some(std::ffi::OsStr::new("1"));
    let cases: Vec<_> = fixtures
        .iter()
        .flat_map(|fixture| cases_for(root, fixture))
        .collect();
    assert!(!cases.is_empty(), "empty UI suite");
    let expected = expected_snapshots(&cases).expect("unique UI snapshot ownership");
    let skipped: BTreeSet<_> = cases
        .iter()
        .filter(|case| {
            case.directives
                .target
                .as_ref()
                .is_some_and(|target| target != &rustc_host())
        })
        .flat_map(case_snapshots)
        .collect();
    let actual = snapshot_files(root).unwrap();
    let (missing, orphan) = snapshot_delta(&expected, &actual);
    assert!(
        (bless && missing.iter().all(|path| !skipped.contains(path))) || missing.is_empty(),
        "missing UI snapshots: {missing:?}"
    );
    assert!(orphan.is_empty(), "orphan UI snapshots: {orphan:?}");
    let staged: Vec<_> = cases
        .iter()
        .flat_map(|case| {
            run_case(
                &case.fixture,
                &case.source,
                &case.directives,
                &case.snapshot,
                bless,
                root,
            )
        })
        .collect();
    if bless {
        for (destination, rendered) in staged {
            fs::write(destination, rendered).unwrap();
        }
    }
    assert_eq!(snapshot_files(root).unwrap(), expected);
}

fn run_case(
    fixture: &Path,
    source: &str,
    directives: &Directives,
    snapshot: &Path,
    bless: bool,
    root: &Path,
) -> Vec<(PathBuf, String)> {
    if let Some(target) = directives
        .target
        .as_ref()
        .filter(|target| *target != &rustc_host())
    {
        eprintln!("UI SKIP {}: only-target={target}", fixture.display());
        return Vec::new();
    }
    let mode = directives.mode.as_deref().unwrap_or("check-pass");
    assert!(
        matches!(mode, "check-pass" | "run-pass" | "compile-fail"),
        "unknown mode {mode}"
    );
    let fixture_temp;
    let cleaned = compiler_source(source);
    let selected_fixture = if cleaned == fs::read_to_string(fixture).unwrap() {
        fixture
    } else {
        fixture_temp = tempfile::Builder::new().suffix(".gin").tempfile().unwrap();
        fs::write(fixture_temp.path(), cleaned).unwrap();
        fixture_temp.path()
    };
    let output = compile(selected_fixture, directives);
    let diagnostics = parse_diagnostics(&output);
    assert_inline(
        source,
        &inline_errors(source).unwrap(),
        &diagnostics,
        fixture,
    );
    assert_eq!(
        output.status.success(),
        mode != "compile-fail",
        "{}: expected {mode}: {}",
        fixture.display(),
        text(&output)
    );
    if mode == "run-pass" {
        assert_runtime(selected_fixture, directives);
    }
    let mut staged = vec![(root.join(snapshot), render_diagnostics(&diagnostics))];
    if directives.snapshot_mlir {
        staged.push((
            root.join(snapshot.with_extension("body")),
            recorded_bodies(source),
        ));
        let mlir = emit_mlir(selected_fixture, directives);
        assert!(
            mlir.status.success(),
            "MLIR emission failed: {}",
            text(&mlir)
        );
        let snapshot_mlir = snapshot.with_extension("mlir");
        let rendered_mlir = normalize_mlir(&String::from_utf8_lossy(&mlir.stdout), fixture);
        let destination_mlir = root.join(snapshot_mlir);
        staged.push((destination_mlir, rendered_mlir));
    }
    if !bless {
        compare_snapshots(&staged);
    }
    staged
}

fn assert_runtime(fixture: &Path, directives: &Directives) {
    let run = run_native(fixture, directives);
    assert_eq!(
        run.status.code(),
        Some(directives.exit_code.unwrap_or(0)),
        "run: {}",
        text(&run)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).as_ref(),
        directives.stdout.as_deref().unwrap_or(""),
        "run stdout"
    );
}

fn compare_snapshots(snapshots: &[(PathBuf, String)]) {
    for (destination, rendered) in snapshots {
        let saved = fs::read_to_string(destination).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {}; set GIN_BLESS=1",
                destination.display()
            )
        });
        assert_eq!(
            &saved,
            rendered,
            "snapshot differs: {}",
            destination.display()
        );
    }
}

fn recorded_bodies(source: &str) -> String {
    let syntax = parser::parse_strict(&compiler_source(source)).expect("snapshot source parses");
    let checked = typecheck::check_file(&syntax, typecheck::Limits::default())
        .expect("snapshot source checks");
    assert_eq!(checked.functions.len(), checked.bodies.len());
    let mut output = String::new();
    for (function, body) in checked.functions.iter().zip(&checked.bodies) {
        output.push_str(&format!("fn {}\n{body:#?}\n", function.name));
        assert!(
            output.len() as u64 <= PROCESS_OUTPUT_LIMIT,
            "recorded body snapshot too large"
        );
    }
    output
}

fn compile(fixture: &Path, directives: &Directives) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
    let package;
    let input = if directives.mode.as_deref() == Some("run-pass") {
        package = ui_package(fixture);
        package.path().to_path_buf()
    } else {
        fixture.to_path_buf()
    };
    command.arg(input);
    command.args(["--emit", "check", "--message-format=json"]);
    if let Some(target) = &directives.target {
        command.args(["--target", target]);
    }
    bounded_output(command)
}

fn emit_mlir(fixture: &Path, directives: &Directives) -> Output {
    let directory = tempfile::tempdir().unwrap();
    let output_path = directory.path().join("fixture.mlir");
    let package;
    let input = if directives.mode.as_deref() == Some("run-pass") {
        package = ui_package(fixture);
        package.path().to_path_buf()
    } else {
        fixture.to_path_buf()
    };
    let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
    command.args(["--emit", "mlir", "--message-format=json"]);
    command.arg(input);
    command.arg("--output").arg(&output_path);
    if let Some(target) = &directives.target {
        command.args(["--target", target]);
    }
    let mut output = bounded_output(command);
    if output.status.success() {
        output.stdout = fs::read(output_path).expect("read emitted MLIR");
    }
    output
}

fn normalize_mlir(mlir: &str, fixture: &Path) -> String {
    mlir.replace(&fixture.to_string_lossy().to_string(), "<fixture>")
}

fn run_native(fixture: &Path, directives: &Directives) -> Output {
    let temp = ui_package(fixture);
    let package = temp.path();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ginc"));
    command.arg(package);
    command.args(["--emit", "exe", "--message-format=json"]);
    if let Some(target) = &directives.target {
        command.args(["--target", target]);
    }
    let executable = temp.path().join(if cfg!(windows) {
        "ui_fixture.exe"
    } else {
        "ui_fixture"
    });
    command.args(["--output", executable.to_str().unwrap()]);
    let build = bounded_output(command);
    assert!(
        build.status.success(),
        "native build failed: {}",
        text(&build)
    );
    bounded_output(Command::new(executable))
}

fn ui_package(fixture: &Path) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("main.gin"),
        compiler_source(&fs::read_to_string(fixture).unwrap()),
    )
    .unwrap();
    fs::write(
        temp.path().join("mod.flask"),
        "#settings\nmodule: \"ui_fixture\"\nversion: 0.1.0\ntarget: \"library\"\n",
    )
    .unwrap();
    temp
}

fn bounded_output(mut command: Command) -> Output {
    bounded_output_with_limits(&mut command, PROCESS_TIMEOUT, PROCESS_OUTPUT_LIMIT)
}

fn bounded_output_with_limits(
    command: &mut Command,
    timeout: Duration,
    output_limit: u64,
) -> Output {
    assert!(timeout > Duration::ZERO);
    assert!(output_limit > 0);
    let directory = tempfile::tempdir().expect("create process capture directory");
    let stdout_path = directory.path().join("stdout");
    let stderr_path = directory.path().join("stderr");
    let stdout = File::create(&stdout_path).expect("create stdout capture");
    let stderr = File::create(&stderr_path).expect("create stderr capture");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .env_remove("GIN_PROFILE_JSON")
        .env_remove("GINC_TIMINGS")
        .env_remove("GINC_FAST_LLVM");
    let mut child = command.spawn().expect("spawn compiler");
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("wait for compiler") {
            return Output {
                status,
                stdout: read_capture(&stdout_path, output_limit),
                stderr: read_capture(&stderr_path, output_limit),
            };
        }
        let stdout_bytes = fs::metadata(&stdout_path)
            .expect("stat stdout capture")
            .len();
        let stderr_bytes = fs::metadata(&stderr_path)
            .expect("stat stderr capture")
            .len();
        if stdout_bytes > output_limit || stderr_bytes > output_limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "UI process output exceeded {output_limit} bytes (stdout={stdout_bytes}, stderr={stderr_bytes})"
            );
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("UI process timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read_capture(path: &Path, output_limit: u64) -> Vec<u8> {
    let file = File::open(path).expect("open process capture");
    let mut output = Vec::new();
    file.take(output_limit + 1)
        .read_to_end(&mut output)
        .expect("read process capture");
    assert!(
        output.len() as u64 <= output_limit,
        "UI process output exceeded {output_limit} bytes"
    );
    output
}

fn parse_diagnostics(output: &Output) -> Vec<Diagnostic> {
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "compiler exited abnormally: {:?}",
        output.status
    );
    let success = output.status.success();
    let mut result = Vec::new();
    let text = String::from_utf8(output.stdout.clone()).expect("compiler JSON stdout is not UTF-8");
    let mut summaries = 0_u32;
    for (index, line) in text.lines().enumerate() {
        let value: Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!(
                "invalid JSON on compiler stdout line {}: {error}",
                index + 1
            )
        });
        match value["reason"].as_str() {
            Some("diagnostic") => {
                assert_eq!(summaries, 0, "diagnostic after completion");
                let severity = value["severity"].as_str().expect("diagnostic severity");
                assert!(matches!(severity, "flaw" | "warn" | "help"));
                let code = value["code"]
                    .as_str()
                    .expect("diagnostic code must be a string");
                assert!(!code.is_empty(), "diagnostic code must not be empty");
                let message = value["message"]
                    .as_str()
                    .expect("diagnostic message must be a string");
                let range = value.get("range").expect("diagnostic range field");
                let span = if range.is_null() {
                    "-".to_owned()
                } else {
                    let start = &range["start"];
                    let end = &range["end"];
                    let start_line = start["line"].as_u64().expect("range start line");
                    let start_column = start["column"].as_u64().expect("range start column");
                    let end_line = end["line"].as_u64().expect("range end line");
                    let end_column = end["column"].as_u64().expect("range end column");
                    assert!(start_line > 0 && start_column > 0);
                    assert!((end_line, end_column) >= (start_line, start_column));
                    format!("{start_line}:{start_column}-{end_line}:{end_column}")
                };
                result.push(Diagnostic {
                    severity: severity.to_owned(),
                    code: code.to_owned(),
                    span,
                    message: message.to_owned(),
                });
            }
            Some("check-finished") => {
                summaries += 1;
                assert_eq!(
                    value["success"].as_bool(),
                    Some(success),
                    "check summary/status mismatch"
                );
            }
            Some(reason) => panic!("unexpected compiler JSON reason {reason:?}"),
            None => panic!("compiler JSON line {} has no reason", index + 1),
        }
    }
    assert_eq!(summaries, 1, "expected exactly one check-finished summary");
    assert_eq!(success, !result.iter().any(|d| d.severity == "flaw"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked at"));
    result.sort_by(|a, b| (&a.span, &a.code, &a.message).cmp(&(&b.span, &b.code, &b.message)));
    result
}

fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| format!("{} {} {} {}\n", d.severity, d.code, d.span, d.message))
        .collect()
}

// TODO(ui-annotations): add severity-aware WARNING/HELP line expectations before migrating tests
// that require them. Full diagnostic snapshots already retain those severities.
fn inline_errors(source: &str) -> Result<Vec<(usize, String)>, String> {
    let mut found = Vec::new();
    let parsed = parser::parse(&compiler_source(source))
        .map_err(|error| format!("cannot read annotations: {error:?}"))?;
    for comment in &parsed.file.comments {
        let span = parsed.file.span_table.get(comment.span_id);
        let Some(annotation) = source[span.start()..span.end()].strip_prefix("--~") else {
            continue;
        };
        let prefix = &source[..span.start()];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let standalone = prefix.rsplit('\n').next().unwrap_or("").trim().is_empty();
        if standalone && line == 1 {
            return Err("line 1: ERROR annotation has no preceding source line".into());
        }
        let words: Vec<_> = annotation.split_whitespace().collect();
        if words.len() != 2 || words[0] != "ERROR" {
            return Err(format!("line {line}: expected --~ ERROR <code>"));
        }
        found.push((line - usize::from(standalone), words[1].to_owned()));
    }
    Ok(found)
}

fn compiler_source(source: &str) -> String {
    source
        .split_inclusive('\n')
        .map(|line| {
            if line.trim_start().starts_with("// ui:") {
                blank_line(line)
            } else {
                line.to_owned()
            }
        })
        .collect()
}

fn blank_line(line: &str) -> String {
    line.bytes()
        .map(|byte| if byte == b'\n' { '\n' } else { ' ' })
        .collect()
}

fn assert_inline(
    source: &str,
    expected: &[(usize, String)],
    actual: &[Diagnostic],
    fixture: &Path,
) {
    let mut lines: BTreeMap<_, usize> = BTreeMap::new();
    for diagnostic in actual {
        if diagnostic.severity != "flaw" {
            continue;
        }
        let line = diagnostic
            .span
            .split(':')
            .next()
            .unwrap_or("-")
            .parse::<usize>()
            .unwrap_or(0);
        *lines.entry((line, diagnostic.code.as_str())).or_default() += 1;
    }
    for (line, code) in expected {
        let remaining = lines.entry((*line, code.as_str())).or_default();
        assert!(
            *remaining > 0,
            "{}:{} expected {code}; source={}",
            fixture.display(),
            line,
            source.lines().nth(line - 1).unwrap_or("")
        );
        *remaining -= 1;
    }
    assert!(
        actual.iter().all(|d| d.span == "-" || !d.code.is_empty()),
        "malformed diagnostic"
    );
}

fn directive_line(line: &str) -> Option<&str> {
    line.trim_start()
        .strip_prefix("-- ui:")
        .or_else(|| line.trim_start().strip_prefix("// ui:"))
}

fn revision_gate(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix("--@[")
        .or_else(|| line.trim().strip_prefix("//@["))
}

fn supported_target(target: &str) -> bool {
    [
        typecheck::Target::AARCH64_DARWIN,
        typecheck::Target::X86_64_DARWIN,
        typecheck::Target::AARCH64_LINUX,
        typecheck::Target::X86_64_LINUX,
    ]
    .iter()
    .any(|known| known.triple == target)
}

fn parse_directives(source: &str) -> Result<Directives, String> {
    let mut directives = Directives::default();
    for (index, line) in source.lines().enumerate() {
        let Some(value) = directive_line(line) else {
            continue;
        };
        for token in
            directive_tokens(value).map_err(|error| format!("line {}: {error}", index + 1))?
        {
            if let Some(revisions) = token.strip_prefix("revisions=") {
                for revision in revisions.split(',') {
                    if revision.is_empty()
                        || !revision
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        return Err(format!("line {}: invalid revision {revision:?}", index + 1));
                    }
                    if !directives.revisions.insert(revision.to_owned()) {
                        return Err(format!("line {}: repeated revision {revision}", index + 1));
                    }
                }
            } else if let Some(target) = token.strip_prefix("only-target=") {
                if !supported_target(target)
                    || directives.target.replace(target.to_owned()).is_some()
                {
                    return Err(format!("line {}: invalid or repeated target", index + 1));
                }
            } else if let Some(stdout) = token.strip_prefix("stdout=") {
                if directives.stdout.replace(stdout.to_owned()).is_some() {
                    return Err(format!("line {}: repeated stdout", index + 1));
                }
            } else if let Some(code) = token.strip_prefix("exit-code=") {
                let code = code
                    .parse()
                    .map_err(|_| format!("line {}: invalid exit code", index + 1))?;
                if directives.exit_code.replace(code).is_some() {
                    return Err(format!("line {}: repeated exit code", index + 1));
                }
            } else if token == "snapshot=mlir" {
                if directives.snapshot_mlir {
                    return Err(format!("line {}: repeated snapshot", index + 1));
                }
                directives.snapshot_mlir = true;
            } else if matches!(token.as_str(), "check-pass" | "run-pass" | "compile-fail") {
                if directives.mode.replace(token.to_owned()).is_some() {
                    return Err(format!("line {}: repeated mode", index + 1));
                }
            } else {
                return Err(format!(
                    "line {}: unknown UI directive {token:?}",
                    index + 1
                ));
            }
        }
    }
    if directives.mode.as_deref() != Some("run-pass")
        && (directives.stdout.is_some() || directives.exit_code.is_some())
    {
        return Err("stdout and exit-code require run-pass".to_owned());
    }
    if directives.mode.as_deref() == Some("compile-fail") && directives.snapshot_mlir {
        return Err("compile-fail cannot snapshot MLIR".to_owned());
    }
    if directives
        .exit_code
        .is_some_and(|code| !(0..=255).contains(&code))
    {
        return Err("exit-code must be between 0 and 255".to_owned());
    }
    Ok(directives)
}

fn directive_tokens(text: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in text.chars() {
        if escaped {
            if character == 'n' {
                token.push('\n');
            } else if character == '"' || character == '\\' {
                token.push(character);
            } else {
                return Err(format!("unsupported escape \\{character}"));
            }
            escaped = false;
        } else if character == '\\' && quoted {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(character);
        }
    }
    if quoted || escaped {
        return Err("unterminated quoted directive value".into());
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Ok(tokens)
}

fn revision_source(source: &str, revision: &str) -> String {
    let mut selected = String::new();
    let mut include = true;
    for line in source.split_inclusive('\n') {
        if let Some(gate) = revision_gate(line) {
            if let Some(name) = gate.strip_suffix(']') {
                include = name == revision;
            }
            selected.push_str(&blank_line(line));
        } else if include {
            selected.push_str(line);
        } else {
            selected.push_str(&blank_line(line));
        }
    }
    assert_eq!(selected.len(), source.len());
    assert_eq!(selected.lines().count(), source.lines().count());
    selected
}

fn validate_revision_gates(source: &str, directives: &Directives) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for (index, line) in source.lines().enumerate() {
        let Some(gate) = revision_gate(line) else {
            continue;
        };
        let Some(revision) = gate.strip_suffix(']') else {
            return Err(format!("line {}: malformed revision gate", index + 1));
        };
        if !directives.revisions.contains(revision) {
            return Err(format!(
                "line {}: gate references undeclared revision {revision:?}",
                index + 1
            ));
        }
        seen.insert(revision.to_owned());
    }
    if seen != directives.revisions {
        return Err("each declared revision must have a source gate".to_owned());
    }
    Ok(())
}

fn rustc_host() -> String {
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        let output = bounded_output({
            let mut command = Command::new("rustc");
            command.arg("-vV");
            command
        });
        assert!(output.status.success(), "rustc -vV failed");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .expect("rustc host triple")
            .to_owned()
    })
    .clone()
}

fn files_under(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut visited = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            visited += 1;
            if visited > MAX_FIXTURES * 8 {
                return Err("UI directory entry limit exceeded".into());
            }
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_symlink() {
                return Err(format!(
                    "UI fixtures cannot be symlinks: {}",
                    path.display()
                ));
            }
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                if entry.metadata().map_err(|error| error.to_string())?.len() > 1024 * 1024 {
                    return Err(format!("UI file exceeds 1 MiB: {}", path.display()));
                }
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn discover(root: &Path) -> Result<Vec<PathBuf>, String> {
    let files: Vec<_> = files_under(root)?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "gin"))
        .collect();
    if files.len() > MAX_FIXTURES {
        return Err("UI fixture limit exceeded".into());
    }
    Ok(files)
}

fn snapshot_files(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    Ok(files_under(root)?
        .into_iter()
        .filter(|path| {
            path.extension().is_some_and(|extension| {
                matches!(
                    extension.to_str(),
                    Some("diag" | "body" | "mlir" | "stderr" | "stdout")
                )
            })
        })
        .map(|path| path.strip_prefix(root).unwrap().to_owned())
        .collect())
}

fn snapshot_delta(
    expected: &BTreeSet<PathBuf>,
    actual: &BTreeSet<PathBuf>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let missing = expected.difference(actual).cloned().collect();
    let orphan = actual.difference(expected).cloned().collect();
    (missing, orphan)
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_comments_do_not_remove_code_or_match_string_contents() {
        let source = "value: \"--~ ERROR not-an-annotation // ui:\"\nf(): return missing --~ ERROR source-check-unknown-name\n";
        assert_eq!(compiler_source(source), source);
        assert_eq!(
            inline_errors(source).unwrap(),
            vec![(2, "source-check-unknown-name".to_owned())]
        );
    }

    #[test]
    fn revisions_preserve_original_utf8_offsets_and_require_each_gate() {
        let source = "-- ui: revisions=a,b\n--@[a]\nfirst: \"💣\"\n--@[b]\nsecond: 2\n";
        let selected = revision_source(source, "b");
        assert_eq!(selected.len(), source.len());
        assert_eq!(selected.find("second"), source.find("second"));
        assert!(!selected.contains("first"));
        assert!(
            validate_revision_gates(
                "--@[a]\nfirst: 1\n",
                &parse_directives("-- ui: revisions=a,b\n").unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn check_pass_does_not_imply_an_ir_snapshot() {
        let case = UiCase {
            fixture: PathBuf::from("check.gin"),
            source: String::new(),
            directives: parse_directives("-- ui: check-pass\n").unwrap(),
            snapshot: PathBuf::from("check.diag"),
        };
        assert_eq!(case_snapshots(&case), [PathBuf::from("check.diag")]);
        assert!(parse_directives("-- ui: check-pass exit-code=0").is_err());
        assert!(parse_directives("-- ui: only-target=typo-target").is_err());
        assert!(parse_directives("-- ui: run-pass exit-code=-1").is_err());
    }

    #[test]
    fn revisions_cannot_collide_with_another_fixtures_snapshot() {
        let case = |fixture: &str| UiCase {
            fixture: PathBuf::from(fixture),
            source: String::new(),
            directives: Directives::default(),
            snapshot: PathBuf::from("foo.a.diag"),
        };
        let error = expected_snapshots(&[case("foo.gin"), case("foo.a.gin")]).unwrap_err();
        assert!(error.contains("foo.a.diag"));
        assert!(error.contains("foo.gin") && error.contains("foo.a.gin"));
    }

    #[test]
    fn multiple_error_codes_on_one_line_are_matched_independently() {
        let diagnostics: Vec<_> = ["first-code", "second-code"]
            .into_iter()
            .map(|code| Diagnostic {
                severity: "flaw".to_owned(),
                code: code.to_owned(),
                span: "1:1-1:2".to_owned(),
                message: String::new(),
            })
            .collect();
        assert_inline(
            "x\n",
            &[(1, "first-code".into()), (1, "second-code".into())],
            &diagnostics,
            Path::new("same-line.gin"),
        );
    }

    #[test]
    fn failed_ir_emission_does_not_bless_diagnostics() {
        let root = tempfile::tempdir().unwrap();
        let fixture = root.path().join("wide.gin");
        let source = include_str!("ui/check-only-wide.gin");
        fs::write(&fixture, source).unwrap();
        fs::write(root.path().join("wide.diag"), "sentinel").unwrap();
        let directives = Directives {
            snapshot_mlir: true,
            ..Directives::default()
        };
        let result = std::panic::catch_unwind(|| {
            run_case(
                &fixture,
                source,
                &directives,
                Path::new("wide.diag"),
                true,
                root.path(),
            )
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("wide.diag")).unwrap(),
            "sentinel"
        );
        assert!(!root.path().join("wide.mlir").exists());
    }

    #[test]
    fn unknown_directives_and_bad_revisions_fail_parsing() {
        assert!(parse_directives("// ui: check-pass typo\n").is_err());
        assert!(parse_directives("// ui: revisions=ok,bad-name!\n").is_err());
        assert!(
            validate_revision_gates(
                "--@[unknown]\nvalue() in 0 to 9: return 1\n",
                &parse_directives("// ui: revisions=known\n").unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn mode_and_revision_directives_are_selected_explicitly() {
        let parsed = parse_directives("// ui: run-pass revisions=host,portable\n").unwrap();
        assert_eq!(parsed.mode.as_deref(), Some("run-pass"));
        assert_eq!(
            parsed.revisions,
            BTreeSet::from(["host".into(), "portable".into()])
        );
    }

    #[test]
    fn run_output_directives_accept_quoted_text_and_newlines() {
        let parsed =
            parse_directives("// ui: run-pass exit-code=7 stdout=\"hello world\\nagain\"\n")
                .unwrap();
        assert_eq!(parsed.exit_code, Some(7));
        assert_eq!(parsed.stdout.as_deref(), Some("hello world\nagain"));
        assert!(parse_directives("// ui: run-pass stdout=\"unfinished\n").is_err());
    }

    #[test]
    fn inline_expectations_capture_the_preceding_diagnostic_line() {
        let parsed = inline_errors("main: return missing\n--~ ERROR source-check-unknown-name\n");
        assert_eq!(
            parsed.unwrap(),
            vec![(1, "source-check-unknown-name".into())]
        );
    }

    #[test]
    fn snapshot_tidy_detects_missing_and_orphan_snapshots() {
        let expected =
            BTreeSet::from([PathBuf::from("present.diag"), PathBuf::from("missing.diag")]);
        let actual = BTreeSet::from([PathBuf::from("present.diag"), PathBuf::from("orphan.diag")]);
        let (missing, orphan) = snapshot_delta(&expected, &actual);
        assert_eq!(missing, [PathBuf::from("missing.diag")]);
        assert_eq!(orphan, [PathBuf::from("orphan.diag")]);
    }

    #[test]
    fn inline_annotation_mismatch_is_a_failure() {
        let expected =
            inline_errors("f() in 0 to 9:\n  return missing\n--~ ERROR expected-code\n").unwrap();
        let actual = vec![Diagnostic {
            severity: "flaw".into(),
            code: "source-check-unknown-name".into(),
            span: "2:10-2:17".into(),
            message: "unknown name".into(),
        }];
        assert!(
            std::panic::catch_unwind(|| {
                assert_inline(
                    "f() in 0 to 9:\n  return missing\n",
                    &expected,
                    &actual,
                    Path::new("bad.gin"),
                )
            })
            .is_err()
        );
    }

    #[test]
    fn blessing_does_not_write_when_inline_expectations_fail() {
        let root = tempfile::tempdir().unwrap();
        let fixture = root.path().join("bad.gin");
        let source = "f() in 0 to 9:\n  return missing\n--~ ERROR deliberately-wrong-code\n";
        fs::write(&fixture, source).unwrap();
        let snapshot = Path::new("bad.diag");
        let result = std::panic::catch_unwind(|| {
            run_case(
                &fixture,
                source,
                &Directives {
                    mode: Some("compile-fail".into()),
                    ..Directives::default()
                },
                snapshot,
                true,
                root.path(),
            )
        });
        assert!(result.is_err());
        assert!(!root.path().join(snapshot).exists());
    }
}

#[cfg(unix)]
mod process_protocol_tests {
    use super::*;
    use std::{os::unix::process::ExitStatusExt, process::ExitStatus, time::Instant};

    fn output(status: i32, stdout: &[u8]) -> Output {
        Output {
            status: ExitStatus::from_raw(status),
            stdout: stdout.to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn diagnostic_protocol_rejects_malformed_json_and_missing_summary() {
        assert!(std::panic::catch_unwind(|| parse_diagnostics(&output(0, b"not-json\n"))).is_err());
        let line = br#"{"reason":"diagnostic","code":"x","message":"bad","range":null}"#;
        assert!(std::panic::catch_unwind(|| parse_diagnostics(&output(0, line))).is_err());
        let missing_range = br#"{"reason":"diagnostic","code":"x","message":"bad"}"#;
        assert!(std::panic::catch_unwind(|| parse_diagnostics(&output(0, missing_range))).is_err());
    }

    #[test]
    fn diagnostic_protocol_rejects_status_mismatch_and_abnormal_exit() {
        let mismatch = br#"{"reason":"check-finished","success":false}"#;
        assert!(std::panic::catch_unwind(|| parse_diagnostics(&output(0, mismatch))).is_err());
        let summary = br#"{"reason":"check-finished","success":true}"#;
        assert!(std::panic::catch_unwind(|| parse_diagnostics(&output(9, summary))).is_err());
    }

    #[test]
    fn process_output_cap_kills_a_prolific_child() {
        let result = std::panic::catch_unwind(|| {
            let mut command = Command::new("yes");
            bounded_output_with_limits(&mut command, Duration::from_secs(3), 1024)
        });
        assert!(result.is_err());
    }

    #[test]
    fn process_timeout_kills_and_reaps_a_slow_child() {
        let started = Instant::now();
        let result = std::panic::catch_unwind(|| {
            let mut command = Command::new("sleep");
            command.arg("5");
            bounded_output_with_limits(&mut command, Duration::from_millis(100), 1024)
        });
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
