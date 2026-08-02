use clap::Parser;
use ginc::cli::Args;
use ginc::compile::{CompileFailure, CompileResult, GinCompiler};
use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(debug_assertions)]
    let start = std::time::Instant::now();
    #[cfg(debug_assertions)]
    eprintln!("[ginc] start");

    let mut args = Args::parse();
    let result = GinCompiler::compile(&mut args);

    if let CompileResult::Failed(failure) = result {
        eprintln!("error: {failure}");

        if let CompileFailure::EmissionFailed { diagnostics, .. } = failure
            && diagnostics.is_empty()
        {
            eprintln!("error: no diagnostics were produced");
        }

        #[cfg(debug_assertions)]
        eprintln!("[ginc] failed after {:?}", start.elapsed());

        return ExitCode::FAILURE;
    }

    #[cfg(debug_assertions)]
    eprintln!("[ginc] done ({:.2?})", start.elapsed());

    ExitCode::SUCCESS
}
