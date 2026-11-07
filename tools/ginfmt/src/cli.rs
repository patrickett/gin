//! `ginfmt [--check] [--stdin] [path...]`.
//!
//! Exit status: 0 when everything parsed and (with `--check`) nothing would change; 1 when
//! `--check` found files that would change and nothing else went wrong; 2 on any error: bad
//! usage, an unreadable or unwritable file, a syntax flaw (printed with the renderer that
//! `ginc --emit check` uses, and the file left untouched), or a formatter bug.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use diagnostic::{Diagnostic, DiagnosticPathExt};

use crate::{FormatError, Options};

pub const USAGE: &str = "usage: ginfmt [--check] [--stdin] [path...]
  Formats Gin files in place. A directory is searched for *.gin files.
  --check   write nothing; list the files that would change, one per line
  --stdin   read source from standard input and write the result to standard output
  exit status: 0 done / nothing to change, 1 --check found files to change, 2 error";

#[derive(Default)]
struct Args {
    check: bool,
    stdin: bool,
    help: bool,
    paths: Vec<PathBuf>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut only_paths = false;
    for arg in args {
        if only_paths || !arg.starts_with('-') || arg == "-" {
            parsed.paths.push(PathBuf::from(arg));
            continue;
        }
        match arg.as_str() {
            "--" => only_paths = true,
            "--check" => parsed.check = true,
            "--stdin" => parsed.stdin = true,
            "--help" | "-h" => parsed.help = true,
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    if parsed.stdin && !parsed.paths.is_empty() {
        return Err("--stdin takes no paths".to_owned());
    }
    if !parsed.help && !parsed.stdin && parsed.paths.is_empty() {
        return Err("no paths given".to_owned());
    }
    Ok(parsed)
}

/// Files named, and `*.gin` files found below directories named, in a stable order.
fn collect(path: &Path, found: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            errors.push(format!("{}: {error}", path.display()));
            return;
        }
    };
    if !metadata.is_dir() {
        found.push(path.to_owned());
        return;
    }
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(path) {
        Ok(entries) => entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect(),
        Err(error) => {
            errors.push(format!("{}: {error}", path.display()));
            return;
        }
    };
    entries.sort();
    for entry in entries {
        let name = entry.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        let linked = std::fs::symlink_metadata(&entry).is_ok_and(|m| m.file_type().is_symlink());
        if entry.is_dir() {
            if !name.starts_with('.') && name != "target" && !linked {
                collect(&entry, found, errors);
            }
        } else if entry.extension().is_some_and(|extension| extension == "gin") {
            found.push(entry);
        }
    }
}

/// Print flaws with the compiler's human diagnostic renderer.
fn report(label: &str, source: &str, error: &FormatError, stderr: &mut dyn Write) {
    match error {
        FormatError::Flaws(diagnostics) => {
            let mut sorted: Vec<&Diagnostic> = diagnostics.iter().collect();
            sorted.sort_by(|a, b| {
                (a.span.start(), a.span.end(), a.code.slug(), &a.message).cmp(&(
                    b.span.start(),
                    b.span.end(),
                    b.code.slug(),
                    &b.message,
                ))
            });
            for diagnostic in sorted {
                diagnostic.print(source, label);
            }
            let _ = writeln!(stderr, "ginfmt: {label}: not formatted, {} syntax flaw(s)", diagnostics.len());
        }
        FormatError::Unverified(_) => {
            let _ = writeln!(stderr, "ginfmt: {label}: {error}");
        }
    }
}

pub fn run(
    args: impl IntoIterator<Item = String>,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let args = match parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            let _ = writeln!(stderr, "ginfmt: {message}\n{USAGE}");
            return 2;
        }
    };
    if args.help {
        let _ = writeln!(stdout, "{USAGE}");
        return 0;
    }
    let options = Options::default();
    if args.stdin {
        let mut source = String::new();
        if let Err(error) = stdin.read_to_string(&mut source) {
            let _ = writeln!(stderr, "ginfmt: <stdin>: {error}");
            return 2;
        }
        return match crate::format_with(&source, options) {
            Err(error) => {
                report("<stdin>", &source, &error, stderr);
                2
            }
            Ok(formatted) if args.check => {
                if formatted == source {
                    0
                } else {
                    let _ = writeln!(stdout, "<stdin>");
                    1
                }
            }
            Ok(formatted) => {
                let _ = stdout.write_all(formatted.as_bytes());
                0
            }
        };
    }
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for path in &args.paths {
        collect(path, &mut files, &mut errors);
    }
    let mut failed = !errors.is_empty();
    for message in &errors {
        let _ = writeln!(stderr, "ginfmt: {message}");
    }
    let mut would_change = false;
    for path in &files {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                let _ = writeln!(stderr, "ginfmt: {}: {error}", path.display());
                failed = true;
                continue;
            }
        };
        let label = path.normalize_diagnostic_path().diagnostic_report_path_from_cwd();
        match crate::format_with(&source, options) {
            Err(error) => {
                report(&label, &source, &error, stderr);
                failed = true;
            }
            Ok(formatted) if formatted == source => {}
            Ok(_) if args.check => {
                would_change = true;
                let _ = writeln!(stdout, "{}", path.display());
            }
            Ok(formatted) => {
                if let Err(error) = std::fs::write(path, formatted) {
                    let _ = writeln!(stderr, "ginfmt: {}: {error}", path.display());
                    failed = true;
                }
            }
        }
    }
    if failed {
        2
    } else {
        i32::from(would_change)
    }
}
