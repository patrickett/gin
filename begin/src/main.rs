mod command;
mod dep_cache;
mod flask;
use crate::{command::BeginCommand, flask::FlaskConfig};
use clap::*;

// TODO: So if a semantic version is of the same major version the interface should
// be the same which means begin can use an already compiled one of the same
// major version

#[derive(Parser, Debug)]
#[command(version, about)]
/// Begin is the package manager for the Gin programming language
pub struct BeginArguments {
    #[command(subcommand)]
    command: BeginCommand,
}

fn main() {
    if let Some(config) = FlaskConfig::from_current_directory() {
        match BeginArguments::parse().command.run(config) {
            Ok(_) => {
                // #[cfg(debug_assertions)]
                // println!("");
            }
            Err(ginc_error) => eprintln!("{ginc_error:#?}"),
        };
    }
}
