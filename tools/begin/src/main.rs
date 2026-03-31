mod command;
mod tui;

use crate::{command::BeginCommand, tui::Tui};
use clap::*;
use flask::FlaskConfig;

// TODO: So if a semantic version is of the same major version the interface should
// be the same which means begin can use an already compiled one of the same
// major version
//
// TODO: automatic semantic versioning, when publishing a flask we want to make
// sure via the compiler that the interface is compatible with the version
// via a hash or something similar

#[derive(Parser, Debug)]
#[command(version, about)]
/// Begin is the package manager for the Gin programming language
pub struct BeginArguments {
    // path: PathBuf,
    #[command(subcommand)]
    command: Option<BeginCommand>,
}

fn main() {
    let args = BeginArguments::parse();

    // TODO: come up with a different pattern for this,
    // we want to prompt when no flask
    if let Some(cmd) = &args.command
        && !cmd.needs_config()
    {
        cmd.run(None);
        return;
    }

    let config = FlaskConfig::from_current_directory();

    if let Some(cmd) = &args.command {
        cmd.run(config)
    } else if config.is_some() {
        let terminal = ratatui::init();
        let app_result = Tui::default().run(terminal);
        ratatui::restore();
        match app_result {
            Ok(_) => {}
            Err(e) => eprintln!("{e}"),
        }
    }
}
