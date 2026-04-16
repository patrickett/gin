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
//!     compile_and_run("main: 1 + 2\n")
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
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;

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
/// result.assert_compiled().assert_exit_code(0).assert_stdout_eq("ok\n");
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
    /// Whether the full compilation pipeline (codegen + link) succeeded.
    pub fn compiled(&self) -> bool {
        self.compiled
    }

    /// Whether the executable was actually launched.
    pub fn ran(&self) -> bool {
        self.ran
    }

    /// The process exit code, if the program ran and exited normally.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// The captured standard output of the program.
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// The captured standard error of the program.
    pub fn stderr(&self) -> &str {
        &self.stderr
    }
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

    /// Assert that compilation or linking failed (no executable produced).
    pub fn assert_compile_failed(&self) -> &Self {
        assert!(
            !self.compiled,
            "expected compilation to fail, but an executable was produced.",
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

    /// Assert that the program exited with a non-zero code.
    pub fn assert_failure(&self) -> &Self {
        assert!(
            self.ran,
            "program was not run — compilation may have failed."
        );
        assert!(
            self.exit_code.is_some_and(|c| c != 0),
            "expected non-zero exit code, got {:?}\n\
             stdout:\n{}\n\
             stderr:\n{}",
            self.exit_code,
            self.stdout,
            self.stderr,
        );
        self
    }

    /// Assert that stdout contains `needle`.
    pub fn assert_stdout_contains(&self, needle: &str) -> &Self {
        assert!(self.compiled, "cannot check stdout: compilation failed.");
        assert!(
            self.stdout.contains(needle),
            "expected stdout to contain {needle:?}\n\nactual stdout:\n{}",
            self.stdout,
        );
        self
    }

    /// Assert that stdout does NOT contain `needle`.
    pub fn assert_stdout_not_contains(&self, needle: &str) -> &Self {
        assert!(self.compiled, "cannot check stdout: compilation failed.");
        assert!(
            !self.stdout.contains(needle),
            "expected stdout NOT to contain {needle:?}\n\nactual stdout:\n{}",
            self.stdout,
        );
        self
    }

    /// Assert that stdout equals `expected` exactly.
    pub fn assert_stdout_eq(&self, expected: &str) -> &Self {
        assert!(self.compiled, "cannot check stdout: compilation failed.");
        assert_eq!(
            self.stdout, expected,
            "expected stdout to equal {expected:?}\n\nactual stdout:\n{}",
            self.stdout,
        );
        self
    }

    /// Assert that stderr contains `needle`.
    pub fn assert_stderr_contains(&self, needle: &str) -> &Self {
        assert!(self.compiled, "cannot check stderr: compilation failed.");
        assert!(
            self.stderr.contains(needle),
            "expected stderr to contain {needle:?}\n\nactual stderr:\n{}",
            self.stderr,
        );
        self
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
        ..Default::default()
    };

    GinCompiler::compile(&mut args);

    // The compiler is void — detect success by checking the executable exists.
    if !exe_path.exists() {
        return RunResult {
            compiled: false,
            ran: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
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
