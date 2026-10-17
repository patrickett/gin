#![allow(dead_code)]
//! Compile-run-assert harness for Gin programs.
//!
//!
//! # Usage in integration tests
//!
//! ```rust,ignore
//! mod common;
//! use common::*;
//!
//! #[test]
//! fn test_addition() {
//!     compile_and_run("#entry\nmain: 1 + 2\n")
//!         .assert_compiled()
//!         .assert_exit_code(3);
//! }
//! ```
//!
//! # Pipeline
//!
//! 1. Write source to a temp `.gin` file.
//! 2. Build an `Args` with `emit: Emit::Exe`.
//! 3. Call `GinCompiler::compile(&mut args)`.
//! 4. Check whether the executable appeared on disk.
//! 5. Run it, capturing stdout and stderr.
//!
//! Temporary build artifacts are written to a per-test directory under
//! `std::env::temp_dir()` and cleaned up on `Drop`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::{CompileResult, GinCompiler};

/// Global counter for unique temp directory names (parallel-safe).
static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Configuration for a compile-run-assert invocation.
pub struct Options {
    /// Build profile (Debug or Release). Defaults to Debug.
    pub profile: Profile,
    /// Descriptive name used for temp directory and the `.gin` file.
    /// Defaults to `"gin_test"`.
    pub test_name: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            profile: Profile::Debug,
            test_name: "gin_test".to_string(),
        }
    }
}

/// The outcome of a compile-and-run cycle.
///
/// Provides builder-style assertion methods so tests read fluently:
///
/// ```rust,ignore
/// result.assert_compiled().assert_exit_code(0);
/// ```
pub struct RunResult {
    /// Compilation + linking succeeded and the executable exists on disk.
    compiled: bool,
    /// The executable was spawned and waited on.
    ran: bool,
    /// Process exit code (if it ran and exited normally).
    exit_code: Option<i32>,
    /// Captured standard output of the program.
    stdout: String,
    /// Captured standard error of the program.
    stderr: String,
    /// Temporary directory holding build artifacts; removed on drop.
    temp_dir: PathBuf,
}

impl RunResult {
    /// Assert that compilation and linking succeeded.
    ///
    /// Panics with a diagnostic message if the executable was not produced.
    pub fn assert_compiled(&self) -> &Self {
        assert!(
            self.compiled,
            "expected compilation to succeed, but no executable was produced.\n\
             stderr:\n{}",
            self.stderr,
        );
        self
    }

    /// Assert that the program ran and exited with `expected`.
    pub fn assert_exit_code(&self, expected: i32) -> &Self {
        assert!(
            self.ran,
            "program was not run — compilation may have failed."
        );
        assert_eq!(
            self.exit_code,
            Some(expected),
            "expected exit code {expected}, got {:?}\n\
             stdout:\n{}\n\
             stderr:\n{}",
            self.exit_code,
            self.stdout,
            self.stderr,
        );
        self
    }

    /// Assert that the program exited with code 0.
    pub fn assert_success(&self) -> &Self {
        self.assert_exit_code(0)
    }
}

impl Drop for RunResult {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

/// Compile and run Gin source code with default options.
pub fn compile_and_run(source: &str) -> RunResult {
    compile_and_run_with_options(source, Options::default())
}

/// Compile and run Gin source code with custom options.
///
/// The temp directory is cleaned up when the returned [`RunResult`] is dropped.
pub fn compile_and_run_with_options(source: &str, opts: Options) -> RunResult {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("gin_cra_{}_{}", opts.test_name, id));
    // Ensure a clean slate if a stale directory lingered.
    let _ = fs::remove_dir_all(&temp_dir);
    let _ = fs::create_dir_all(&temp_dir);

    let source_path = temp_dir.join(format!("{}.gin", opts.test_name));
    let exe_path = temp_dir.join(&opts.test_name);

    // Write source to temp file.
    if let Err(err) = fs::write(&source_path, source) {
        return RunResult {
            compiled: false,
            ran: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("failed to write source file: {err}"),
            temp_dir,
        };
    }

    // Build Args and invoke the production compiler pipeline.
    let mut args = Args {
        input: source_path,
        emit: Emit::Exe,
        output: Some(exe_path.clone()),
        profile: opts.profile,
        target: Some(test_fixtures::host_target_triple()),
        ..Default::default()
    };

    let result = GinCompiler::compile(&mut args);

    // The compiler is void — detect success by checking the executable exists.
    if !result.is_success() || !exe_path.exists() {
        return RunResult {
            compiled: false,
            ran: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("{result:?}"),
            temp_dir,
        };
    }

    // Run the executable and capture output.
    match Command::new(&exe_path).output() {
        Ok(output) => RunResult {
            compiled: true,
            ran: true,
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            temp_dir,
        },
        Err(err) => RunResult {
            compiled: true,
            ran: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("failed to execute: {err}"),
            temp_dir,
        },
    }
}

pub fn compile_exe(input: PathBuf, output: &Path, profile: Profile) -> CompileResult {
    let mut args = Args {
        input,
        output: Some(output.to_path_buf()),
        emit: Emit::Exe,
        profile,
        target: Some(test_fixtures::host_target_triple()),
        no_cache: true,
        ..Default::default()
    };
    GinCompiler::compile(&mut args)
}

pub fn require_success(result: &CompileResult, profile: Profile) {
    assert!(result.is_success(), "{profile:?}: {result:?}");
}

pub fn compile_exe_checked(input: PathBuf, output: &Path, profile: Profile) {
    let result = compile_exe(input, output, profile);
    require_success(&result, profile);
    assert!(
        !result.has_typecheck_diagnostics(),
        "{profile:?}: {result:?}"
    );
}

pub fn compile_exe_with_gin_home(
    input: PathBuf,
    output: &Path,
    gin_home: PathBuf,
) -> CompileResult {
    let mut args = Args {
        input,
        output: Some(output.to_path_buf()),
        target: Some(test_fixtures::host_target_triple()),
        no_cache: true,
        gin_home: Some(flask::GinHome::new(gin_home)),
        ..Default::default()
    };
    GinCompiler::compile(&mut args)
}

/// Compiles `input`, asserts a clean typecheck, then runs the executable with
/// `stdin` on its standard input. `context` is the panic message used when the
/// child cannot be spawned or waited on.
pub fn compile_and_run_exe(
    input: PathBuf,
    output: &Path,
    profile: Profile,
    stdin: &str,
    context: &str,
) -> Output {
    compile_exe_checked(input, output, profile);
    let mut child = Command::new(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect(context);
    child
        .stdin
        .as_mut()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write child stdin");
    child.wait_with_output().expect(context)
}

/// Compile and run `input` under both profiles, asserting each run succeeds.
pub fn run_in_profiles(package: &test_fixtures::TempPackage, input: PathBuf, context: &str) {
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("program-{profile:?}"));
        let run = compile_and_run_exe(input.clone(), &executable, profile, "", context);
        assert!(run.status.success(), "{profile:?}: {run:?}");
    }
}

/// Shorthand: compile-and-run with a unique test name.
pub fn cra(name: &str, source: &str) -> RunResult {
    compile_and_run_with_options(
        source,
        Options {
            test_name: name.to_string(),
            ..Default::default()
        },
    )
}
