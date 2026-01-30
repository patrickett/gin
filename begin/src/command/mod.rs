use crate::command::{
    build::begin_build,
    doc::{DocCommand, begin_doc},
    version::{VersionCommand, version},
};
use clap::*;
use flask::FlaskConfig;
use std::path::PathBuf;

mod build;
mod doc;
mod version;

#[derive(Subcommand, Debug)]
pub enum BeginCommand {
    // begin add pkg_name
    // begin add "*.git" for git
    Add,

    /// Runs a security audit on the dependencies of the current project
    Audit,

    /// Run the benchmarks declared in the current project
    Bench,

    /// Compile the specified module (Default: cwd)
    #[command(alias = "b")]
    Build {
        path: Option<PathBuf>,
        watch: Option<bool>,
    },

    /// Analyze the current module and report errors, but don't build object files
    #[command(alias = "c")]
    Check {},

    /// Generate documentation for current package and its dependencies
    #[command(subcommand, alias = "d")]
    Doc(DocCommand),

    /// Format all the tracked files in the current package with 'ginfmt'
    Format,

    /// Run the current project, will just compile a library if no entry
    Run,

    /// Run the tests declared in the current project
    Test,

    #[command(subcommand, alias = "v")]
    Version(VersionCommand),
    // TODO: begin info {pkg}
    //     - list info for the package
    //     - optionally show dependencies
    //     - https://docs.deno.com/runtime/reference/cli/info/
    //     - show cache location
    // Info,
}

impl BeginCommand {
    pub fn run(&self, config: FlaskConfig) {
        match &self {
            BeginCommand::Build { path: input, .. } => begin_build(config, input.to_owned()),
            BeginCommand::Doc(_cmd) => begin_doc(config),
            BeginCommand::Version(cmd) => version(cmd),
            _ => todo!(),
        }
    }
}
