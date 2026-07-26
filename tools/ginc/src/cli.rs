use clap::{Parser, ValueEnum};
pub use codegen::emit::Profile;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum Emit {
    /// Produce a native executable (default)
    #[default]
    Exe,
    /// Produce an object file only
    Obj,
    /// Print MLIR text to stdout
    Mlir,
}

#[derive(Parser, Debug, Default)]
#[command(version, about)]
pub struct Args {
    pub input: PathBuf,

    /// Write output to <OUTPUT>
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Target triple for cross-compilation
    // TODO: add a better struct for this
    #[arg(long)]
    pub target: Option<String>,

    /// What to emit
    #[arg(long, default_value = "exe")]
    pub emit: Emit,

    /// Build profile
    #[arg(long, default_value = "debug")]
    pub profile: Profile,

    /// Disable the compilation cache
    #[arg(long)]
    pub no_cache: bool,

    /// Print per-phase compilation timings.
    ///
    /// Also enabled when `GINC_TIMINGS` is set to `1`, `true`, or `on`.
    #[arg(long)]
    pub timings: bool,

    /// Use a faster LLVM/MLIR path for development builds (disable MLIR optimize pipeline,
    /// compile LLVM IR with `-O0`).
    #[arg(long)]
    pub fast_llvm: bool,

    #[arg(short, long)]
    pub verbose: Option<bool>,

    /// Resolved dependency paths: name → directory. Populated by `begin build` from flask.jsonc.
    /// Not a CLI argument — set programmatically.
    #[arg(skip)]
    pub dependencies: HashMap<String, PathBuf>,
}

impl Args {
    pub fn folder_name(&self) -> String {
        self.input
            .file_name()
            .unwrap_or(OsStr::new("project"))
            .to_string_lossy()
            .to_string()
    }
}
