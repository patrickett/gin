mod command;

use crate::command::BeginCommand;
use clap::*;
use flask::FlaskConfig;

#[derive(Parser, Debug)]
#[command(version, about, arg_required_else_help = true)]
/// Begin is the package manager for the Gin programming language
pub struct BeginArguments {
    #[command(subcommand)]
    command: BeginCommand,
}

fn main() {
    let args = BeginArguments::parse();

    if !args.command.needs_preloaded_config() {
        args.command.run(None);
        return;
    }

    let config = match std::env::current_dir() {
        Ok(dir) => match FlaskConfig::discover(&dir) {
            Ok(found) => found.map(|(config, _)| config),
            Err(error) => {
                eprintln!("begin: {error}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("error: cannot get current directory: {e}");
            None
        }
    };

    args.command.run(config);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_displays_help() {
        let error = BeginArguments::try_parse_from(["begin"]).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }
}
