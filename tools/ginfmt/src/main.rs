use flask::{CompileTarget, TargetTriple};
use ginfmt::Config;
use std::path::Path;

const USAGE: &str = "usage: ginfmt [--suggest | --fix] [--target <triple>] [--width <columns>] [--type-alignment-padding <spaces>] <file.gin>...\n  --suggest  show help without changing files\n  --fix      apply verified help to files, preserving surrounding formatting";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Format,
    Suggest,
    Fix,
}

fn run() -> Result<(), String> {
    let mut config = Config::default();
    let mut paths = Vec::new();
    let mut mode = Mode::Format;
    let mut target = CompileTarget::Library;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--suggest" | "--fix" => {
                if mode != Mode::Format {
                    return Err("choose either --suggest or --fix".to_owned());
                }
                mode = if arg == "--fix" {
                    Mode::Fix
                } else {
                    Mode::Suggest
                };
            }
            "--target" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --target".to_owned())?;
                target = CompileTarget::Concrete(
                    TargetTriple::parse(&value).map_err(|error| error.to_string())?,
                );
            }
            "--width" | "--type-alignment-padding" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("missing value for {arg}"))?;
                let value = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid value for {arg}: {value}"))?;
                if arg == "--width" {
                    if value == 0 {
                        return Err("--width must be greater than zero".to_owned());
                    }
                    config.max_line_width = value;
                } else {
                    config.max_type_alignment_padding = value;
                }
            }
            "--" => {
                paths.extend(args);
                break;
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ => paths.push(arg),
        }
    }
    if paths.is_empty() {
        return Err(USAGE.to_owned());
    }
    for path in paths {
        let source = std::fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))?;
        match mode {
            Mode::Format => print!("{}", ginfmt::format_with_config(&source, config.clone())),
            Mode::Suggest => {
                for help in ginfmt::help::suggestions(Path::new(&path), target.clone())? {
                    let (line, _) =
                        diagnostic::source_position(&source, help.diagnostic.span.start());
                    println!(
                        "{path}:{line}: help[{}]: {}\n  {}",
                        help.diagnostic.code.slug(),
                        help.diagnostic.message,
                        help.diagnostic.help.as_deref().unwrap_or_default()
                    );
                }
            }
            Mode::Fix => {
                let count = ginfmt::help::fix(Path::new(&path), target.clone())?;
                eprintln!("{path}: applied {count} fixes");
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("ginfmt: {error}");
        std::process::exit(1);
    }
}
