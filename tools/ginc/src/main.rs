use clap::Parser;
use ginc::{Args, GinCompiler};

fn main() {
    #[cfg(debug_assertions)]
    let start = std::time::Instant::now();
    #[cfg(debug_assertions)]
    eprintln!("[ginc] start");

    let mut args = Args::parse();
    GinCompiler::compile(&mut args);

    #[cfg(debug_assertions)]
    eprintln!("[ginc] done ({:.2?})", start.elapsed());
}
