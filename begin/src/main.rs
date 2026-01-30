mod command;
mod dep_cache;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub static APP_VERSION: &str = VERSION;

pub mod tui;

use crate::{command::BeginCommand, tui::App};
use clap::*;
use flask::FlaskConfig;

// TODO: So if a semantic version is of the same major version the interface should
// be the same which means begin can use an already compiled one of the same
// major version

#[derive(Parser, Debug)]
#[command(version, about)]
/// Begin is the package manager for the Gin programming language
pub struct BeginArguments {
    // path: PathBuf,
    #[command(subcommand)]
    command: Option<BeginCommand>,
}

fn main() {
    match BeginArguments::parse().command {
        Some(cmd) => {
            if let Some(config) = FlaskConfig::from_current_directory() {
                cmd.run(config);
            }
        }
        None => {
            let terminal = ratatui::init();
            let app_result = App::default().run(terminal);
            ratatui::restore();
            match app_result {
                Ok(_) => {}
                Err(e) => eprintln!("{e}"),
            }
        }
    }
}
