use clap::{Parser, ValueEnum};
use std::path::PathBuf;

pub use backend::Profile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum Emit {
    /// Build an executable when the selected module has an entry; otherwise check only.
    #[default]
    Auto,
    /// Produce a native executable.
    Exe,
    /// Produce one object file per module in an output directory.
    Obj,
    /// Emit MLIR text.
    Mlir,
    /// Emit the checked package interface.
    Interface,
    /// Check without emission.
    Check,
}

#[derive(Parser, Debug, Default)]
#[command(version, about)]
pub struct Args {
    /// A package directory, module directory, or Gin source file.
    pub input: PathBuf,
    #[arg(long, value_enum, default_value = "auto")]
    pub emit: Emit,
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Target triple; defaults to the package target or host.
    #[arg(long)]
    pub target: Option<String>,
    #[arg(long, default_value = "debug")]
    pub profile: Profile,
    #[arg(long, value_enum, default_value = "human")]
    pub message_format: crate::reporting::MessageFormat,
    /// Maximum errors to display; omitted errors still cause failure.
    #[arg(long)]
    pub max_errors: Option<usize>,
    #[arg(long, value_enum, default_value = "own")]
    pub warnings: crate::reporting::WarningScope,
    /// Disable artifact reuse. Has no effect when no cache is available.
    #[arg(long)]
    pub no_cache: bool,
    /// Print per-phase timings, also enabled by GINC_TIMINGS.
    #[arg(long)]
    pub timings: bool,
    /// Write JSON timings and suppress build status.
    #[arg(long)]
    pub timings_json: Option<PathBuf>,
    /// Suppress build status, but retain diagnostics and requested timings.
    #[arg(short, long)]
    pub quiet: bool,
    /// Skip MLIR optimization and compile LLVM IR with -O0.
    #[arg(long)]
    pub fast_llvm: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reporting::MessageFormat;

    #[test]
    fn ordinary_commands_select_the_compiler_without_a_stack_flag() {
        let args = Args::try_parse_from(["ginc", "package"]).unwrap();
        assert_eq!(args.emit, Emit::Auto);
        assert_eq!(args.profile, Profile::Debug);
        assert!(args.output.is_none());
    }

    #[test]
    fn all_emission_modes_accept_diagnostic_options() {
        for mode in ["auto", "exe", "obj", "mlir", "check", "interface"] {
            let args = Args::try_parse_from([
                "ginc",
                "package",
                "--emit",
                mode,
                "--message-format=json",
                "--max-errors=2",
                "--warnings=deps",
            ])
            .unwrap();
            assert_eq!(args.message_format, MessageFormat::Json);
            assert_eq!(args.max_errors, Some(2));
        }
    }

    #[test]
    fn removed_stack_selection_flags_are_not_accepted() {
        for arguments in [
            vec!["ginc", "package", "--check-new"],
            vec!["ginc", "package", "--emit-new", "exe"],
            vec!["ginc", "package", "--legacy"],
        ] {
            assert!(Args::try_parse_from(arguments).is_err());
        }
    }
}
