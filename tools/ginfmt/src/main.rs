fn main() {
    let code = ginfmt::cli::run(
        std::env::args().skip(1),
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::exit(code);
}
