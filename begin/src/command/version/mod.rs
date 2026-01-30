use clap::Subcommand;

mod bump;
use bump::*;

#[derive(Subcommand, Debug)]
pub enum VersionCommand {
    /// Check the current module interface and increase
    /// the module version accordingly
    #[command(alias = "b")]
    Bump,
}

pub fn version(cmd: &VersionCommand) {
    match cmd {
        VersionCommand::Bump => bump(),
    }
}
