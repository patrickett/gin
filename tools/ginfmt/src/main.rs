use ginfmt::Config;

fn run() -> Result<(), String> {
    let mut config = Config::default();
    let mut paths = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
        return Err(
            "usage: ginfmt [--width <columns>] [--type-alignment-padding <spaces>] <file.gin>..."
                .to_owned(),
        );
    }
    for path in paths {
        let source = std::fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))?;
        print!("{}", ginfmt::format_with_config(&source, config.clone()));
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("ginfmt: {error}");
        std::process::exit(1);
    }
}
