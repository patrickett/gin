//! Opt-in corpus comparison against a separately supplied compiler, never a simulated baseline.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const FILES_MAX: usize = 20_000;
const OUTPUT_BYTES_MAX: u64 = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

fn inputs(roots: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut pending = roots.to_vec();
    let mut sources = BTreeSet::new();
    let mut packages = BTreeSet::new();
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        visited += 1;
        if visited > FILES_MAX {
            return Err(format!("corpus exceeds {FILES_MAX} entries"));
        }
        let metadata =
            fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            if path.join("mod.flask").is_file() {
                packages.insert(fs::canonicalize(&path).map_err(|e| e.to_string())?);
            }
            let entries = fs::read_dir(&path).map_err(|e| e.to_string())?;
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name();
                if name.to_string_lossy().starts_with('.')
                    || matches!(name.to_str(), Some("target" | "node_modules"))
                {
                    continue;
                }
                pending.push(entry.path());
            }
        } else if path.extension().is_some_and(|e| e == "gin") {
            sources.insert(fs::canonicalize(path).map_err(|e| e.to_string())?);
        }
    }
    let mut units = packages.clone();
    units.extend(sources.into_iter().filter(|source| {
        !source
            .ancestors()
            .any(|ancestor| packages.contains(ancestor))
    }));
    if units.is_empty() {
        return Err("corpus contains no packages or Gin sources".to_owned());
    }
    Ok(units.into_iter().collect())
}

fn read_output(file: &fs::File) -> Result<String, String> {
    if file.metadata().map_err(|e| e.to_string())?.len() > OUTPUT_BYTES_MAX {
        return Err("compiler output exceeds corpus limit".to_owned());
    }
    use std::io::{Read, Seek};
    let mut file = file.try_clone().map_err(|e| e.to_string())?;
    file.rewind().map_err(|e| e.to_string())?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|e| e.to_string())?;
    Ok(text)
}

fn compile(compiler: &Path, arguments: &[String], input: &Path) -> Result<(i32, String), String> {
    let stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
    let stderr = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut child = Command::new(compiler)
        .args(arguments)
        .arg(input)
        .env_remove("GIN_PROFILE_JSON")
        .env_remove("GINC_TIMINGS")
        .env_remove("GINC_FAST_LLVM")
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().map_err(|e| e.to_string())?)
        .stderr(stderr.try_clone().map_err(|e| e.to_string())?)
        .spawn()
        .map_err(|e| format!("{}: {e}", compiler.display()))?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        let oversized = stdout.metadata().map_err(|e| e.to_string())?.len() > OUTPUT_BYTES_MAX
            || stderr.metadata().map_err(|e| e.to_string())?.len() > OUTPUT_BYTES_MAX;
        if start.elapsed() >= TIMEOUT || oversized {
            child.kill().map_err(|e| e.to_string())?;
            child.wait().map_err(|e| e.to_string())?;
            return Err(format!(
                "compiler exceeded time/output limit: {}",
                input.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let code = status.code().ok_or("compiler terminated by signal")?;
    if !matches!(code, 0 | 1) {
        return Err(format!("compiler exited {code}: {}", read_output(&stderr)?));
    }
    Ok((code, read_output(&stdout)?))
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn diagnostics(stdout: &str, code: i32, root: &Path) -> Result<Value, String> {
    let mut diagnostics = Vec::new();
    let mut finished = None;
    for line in stdout.lines() {
        let record: Value = serde_json::from_str(line)
            .map_err(|e| format!("compiler must support JSON diagnostics: {e}: {line}"))?;
        match record["reason"].as_str() {
            Some("diagnostic") => {
                for key in ["code", "severity", "message"] {
                    if !record[key].is_string() {
                        return Err(format!("diagnostic missing string `{key}`: {record}"));
                    }
                }
                let file = match record.get("file") {
                    Some(Value::String(file)) => json!(relative(Path::new(file), root)),
                    Some(Value::Null) => Value::Null,
                    _ => return Err(format!("diagnostic has invalid file: {record}")),
                };
                diagnostics.push(json!({
                    "code": record["code"],
                    "severity": record["severity"],
                    "file": file,
                    "range": record["range"],
                    "message": record["message"],
                    "dependency": record["dependency"],
                }));
            }
            Some("check-finished") => {
                if finished.replace(record["success"].clone()).is_some() {
                    return Err("compiler emitted multiple check-finished records".to_owned());
                }
            }
            _ => return Err(format!("unexpected compiler JSON record: {record}")),
        }
    }
    if finished != Some(json!(code == 0)) {
        return Err("missing check-finished or inconsistent compiler exit status".to_owned());
    }
    diagnostics.sort_by_cached_key(Value::to_string);
    Ok(json!({"success": code == 0, "diagnostics": diagnostics}))
}

fn arguments(variable: &str) -> Result<Vec<String>, String> {
    match std::env::var(variable) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{variable}: {e}")),
        Err(std::env::VarError::NotPresent) => Ok([
            "--emit",
            "check",
            "--message-format=json",
            "--warnings=deps",
            "--quiet",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()),
        Err(error) => Err(format!("{variable}: {error}")),
    }
}

/// `GIN_CORPUS_BASELINE=/path/to/old/ginc cargo test -p ginc --test corpus -- --ignored --nocapture`
#[test]
#[ignore = "opt-in corpus scan; an old compiler must be supplied for differential comparison"]
fn corpus_diagnostics() {
    run_corpus().unwrap_or_else(|error| panic!("{error}"));
}

fn run_corpus() -> Result<(), String> {
    let root = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .map_err(|e| e.to_string())?;
    let roots: Vec<PathBuf> = std::env::var_os("GIN_CORPUS_PATHS").map_or_else(
        || vec![root.join("modules"), root.join("tools/ginc/tests/fixtures")],
        |paths| std::env::split_paths(&paths).collect(),
    );
    let compiler = std::env::var_os("GIN_CORPUS_COMPILER")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_ginc")), PathBuf::from);
    let baseline = std::env::var_os("GIN_CORPUS_BASELINE").map(PathBuf::from);
    let arguments = arguments("GIN_CORPUS_ARGS")?;
    let baseline_arguments = self::arguments("GIN_CORPUS_BASELINE_ARGS")?;
    let mut reports = Vec::new();
    let mut changed = 0;
    for input in inputs(&roots)? {
        let (code, stdout) = compile(&compiler, &arguments, &input)?;
        let current = diagnostics(&stdout, code, &root)?;
        let previous = if let Some(baseline) = &baseline {
            let (code, stdout) = compile(baseline, &baseline_arguments, &input)?;
            Some(diagnostics(&stdout, code, &root)?)
        } else {
            None
        };
        let differs = previous.as_ref().is_some_and(|old| old != &current);
        changed += usize::from(differs);
        reports.push(json!({
            "input": relative(&input, &root), "current": current,
            "baseline": previous, "changed": differs,
        }));
    }
    let report = serde_json::to_string_pretty(&json!({
        "compared": baseline.is_some(), "changed": changed, "units": reports,
    }))
    .map_err(|e| e.to_string())?;
    match std::env::var_os("GIN_CORPUS_REPORT") {
        Some(path) => fs::write(path, report).map_err(|e| e.to_string())?,
        None => println!("{report}"),
    }
    if changed != 0 {
        return Err(format!("{changed} corpus units have changed diagnostics"));
    }
    Ok(())
}

#[test]
fn corpus_discovery_checks_packages_once_and_rejects_empty_inputs() {
    let dir = tempfile::tempdir().unwrap();
    assert!(inputs(&[dir.path().to_owned()]).is_err());
    fs::create_dir(dir.path().join("app")).unwrap();
    fs::write(dir.path().join("app/mod.flask"), "").unwrap();
    fs::write(dir.path().join("app/main.gin"), "").unwrap();
    fs::write(dir.path().join("alone.gin"), "").unwrap();
    let units = inputs(&[dir.path().to_owned()]).unwrap();
    assert_eq!(units.len(), 2);
    assert!(units.iter().any(|p| p.ends_with("app")));
    assert!(units.iter().any(|p| p.ends_with("alone.gin")));
}

#[test]
fn real_compiler_corpus_records_success_failure_and_identical_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let args = arguments("GIN_CORPUS_TEST_ARGS").unwrap();
    let compiler = Path::new(env!("CARGO_BIN_EXE_ginc"));
    for (source, succeeds) in [
        ("answer() in 0 to 9: return 3\n", true),
        ("answer() in 0 to 1: return 3\n", false),
    ] {
        let input = dir.path().join("test.gin");
        fs::write(&input, source).unwrap();
        let (code, stdout) = compile(compiler, &args, &input).unwrap();
        let actual = diagnostics(&stdout, code, dir.path()).unwrap();
        assert_eq!(actual["success"], succeeds);
        let (code, stdout) = compile(compiler, &args, &input).unwrap();
        assert_eq!(actual, diagnostics(&stdout, code, dir.path()).unwrap());
        assert_ne!(actual, json!({"success": !succeeds, "diagnostics": []}));
    }
}

#[test]
fn malformed_or_crashed_compilers_are_not_diagnostic_baselines() {
    assert!(diagnostics("", 1, Path::new(".")).is_err());
    assert!(diagnostics("not json", 0, Path::new(".")).is_err());
    assert!(
        diagnostics(
            "{\"reason\":\"check-finished\",\"success\":false}",
            0,
            Path::new(".")
        )
        .is_err()
    );
    assert!(diagnostics("{\"reason\":\"diagnostic\"}", 1, Path::new(".")).is_err());
}
