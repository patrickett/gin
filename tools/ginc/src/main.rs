use clap::Parser;
use ginc::cli::Args;
use ginc::compile::GinCompiler;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = Args::parse();
    if GinCompiler::compile_cli(&mut args).is_success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
