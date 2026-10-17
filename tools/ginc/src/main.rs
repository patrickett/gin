use clap::Parser;
use ginc::cli::Args;
use ginc::compile::GinCompiler;
use std::process::ExitCode;

#[cfg(feature = "profile-allocations")]
#[global_allocator]
static ALLOCATOR: compiler_profile::TrackingAllocator<std::alloc::System> =
    compiler_profile::TrackingAllocator(std::alloc::System);

fn main() -> ExitCode {
    let Some(path) = std::env::var_os("GIN_PROFILE_JSON") else {
        return compile();
    };
    let (result, report) = compiler_profile::capture(compile);
    let write = serde_json::to_vec_pretty(&report)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&path, bytes));
    if let Err(error) = write {
        eprintln!("could not write compiler profile: {error}");
        return ExitCode::FAILURE;
    }
    result
}

fn compile() -> ExitCode {
    let mut args = Args::parse();
    if GinCompiler::compile_cli(&mut args).is_success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
