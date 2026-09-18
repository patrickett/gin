mod command;
mod tui;

use crate::{command::BeginCommand, tui::Tui};
use clap::*;
use flask::FlaskConfig;

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
        && !cmd.needs_preloaded_config()
    {
        cmd.run(None);
        return;
    }

    let config = match std::env::current_dir() {
        Ok(dir) => FlaskConfig::find_package_config(&dir).map(|(cfg, _)| cfg),
        Err(e) => {
            eprintln!("error: cannot get current directory: {e}");
            None
        }
    };

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
