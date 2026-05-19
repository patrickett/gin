fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: ginfmt <file.gin>");
        std::process::exit(1);
    }

    for path in &args[1..] {
        match std::fs::read_to_string(path) {
            Ok(source) => {
                let formatted = ginfmt::format(&source);
                print!("{formatted}");
            }
            Err(e) => {
                eprintln!("ginfmt: {path}: {e}");
                std::process::exit(1);
            }
        }
    }
}
