use clap::Subcommand;
use std::path::PathBuf;

mod bump;

#[derive(Subcommand, Debug)]
pub enum VersionCommand {
    /// Compare the compiled interface with a previous release and update the version.
    #[command(alias = "b")]
    Bump {
        /// Compare a local interface artifact instead of fetching the signed registry baseline.
        #[arg(long)]
        against: Option<PathBuf>,
        /// Explain the required version without modifying the manifest.
        #[arg(long)]
        check: bool,
    },
}

pub fn version(cmd: &VersionCommand) {
    let result = match cmd {
        VersionCommand::Bump { against, check } => bump::bump(against.as_deref(), *check),
    };
    if let Err(error) = result {
        eprintln!("begin version bump: {error}");
        std::process::exit(1);
    }
}
