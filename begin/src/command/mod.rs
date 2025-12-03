use crate::{
    command::{build::begin_build, doc::begin_doc},
    flask::FlaskConfig,
};
use clap::*;
use ginc::GincResult;
use std::path::PathBuf;

mod build;
mod doc;

#[derive(Subcommand, Debug)]
pub enum VersionCommand {
    /// Check the current module interface and increase
    /// the module version accordingly
    #[command(alias = "b")]
    Bump {
        // input: Option<PathBuf>,
        // watch: Option<bool>,
    },
}

#[derive(Subcommand, Debug, Default)]
pub enum DocCommand {
    /// Opens the docs in a browser after the building
    #[command(alias = "o")]
    #[default]
    Open,
}

#[derive(Subcommand, Debug)]
pub enum BeginCommand {
    /// Compile the specified module (Default: cwd)
    #[command(alias = "b")]
    Build {
        path: Option<PathBuf>,
        watch: Option<bool>,
    },
    /// Analyze the current module and report errors, but don't build object files
    #[command(alias = "c")]
    Check {},
    /// Build this module and its dependencies' documentation
    #[command(subcommand, alias = "d")]
    Doc(DocCommand),

    #[command(subcommand, alias = "v")]
    Version(VersionCommand),
}

impl BeginCommand {
    pub fn run(&self, config: FlaskConfig) -> GincResult<()> {
        let warnings = Vec::new();

        match &self {
            BeginCommand::Build { path: input, .. } => begin_build(config, input.to_owned()),
            BeginCommand::Doc(_cmd) => begin_doc(config),
            BeginCommand::Version(ver) => match ver {
                VersionCommand::Bump {} => {
                    println!("bumping version");
                    Ok((warnings, ()))
                }
            },
            _ => todo!(),
        }
    }
}
