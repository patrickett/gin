use clap::Parser;
use ginc::{Args, GinCompiler};

fn main() {
    let mut args = Args::parse();
    GinCompiler::compile(&mut args);
}
