mod command;
mod dep_cache;
mod tui;

use crate::{command::BeginCommand, tui::Tui};
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
    let Some(config) = FlaskConfig::from_current_directory() else {
        // TODO: create a nice input for asking if they want to init
        return;
    };

    if let Some(cmd) = BeginArguments::parse().command {
        cmd.run(config)
    } else {
        let terminal = ratatui::init();
        let app_result = Tui::default().run(terminal);
        ratatui::restore();
        match app_result {
            Ok(_) => {}
            Err(e) => eprintln!("{e}"),
        }
    }
}
