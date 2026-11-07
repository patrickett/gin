use flask::FlaskConfig;
use ginc::cli::{Args, Emit};
use ginc::compile::{CompileResult, GinCompiler};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

// TODO: support passing args to the executable: begin run -- --some-arg

/// Compile and execute the package's default entry or the supplied source.
pub fn begin_run(
    config: FlaskConfig,
    input: Option<PathBuf>,
    watch: bool,
    sandbox: flask::ExecutionSandbox,
) {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => {
            eprintln!("error: Failed to get current directory");
            return;
        }
    };

    let root = FlaskConfig::find_package_config(&cwd)
        .map(|(_, root)| root)
        .unwrap_or(cwd);
    let path = match input {
        Some(p) => p,
        None => root,
    };

    if !path.exists() {
        eprintln!("error: Entry file not found: {}", path.display());
        std::process::exit(1);
    }

    eprintln!("Compiling {} v{}", config.name, config.version);
    let mut args = Args {
        input: path,
        emit: Emit::Exe,
        ..Default::default()
    };
    let exe_path = match GinCompiler::compile(&mut args) {
        CompileResult::Success { output: Some(path) } => path,
        _ => {
            eprintln!("error: Compilation failed");
            std::process::exit(1);
        }
    };

    eprintln!("Running `{}`", exe_path.display());

    let status = match run_in_sandbox(&exe_path, sandbox) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    };

    if !status.success() {
        eprintln!("info: Exited with status: {}", status);
        std::process::exit(status.code().unwrap_or(1));
    }

    // TODO: implement watch mode for recompile+run loop
    if watch {
        eprintln!("warning: Watch mode not yet implemented - run once");
    }
}

fn run_in_sandbox(
    executable: &Path,
    sandbox: flask::ExecutionSandbox,
) -> Result<ExitStatus, String> {
    let scratch =
        tempfile::tempdir().map_err(|error| format!("cannot create execution sandbox: {error}"))?;
    let mut command = sandbox
        .command(executable, scratch.path())
        .map_err(|error| format!("cannot configure execution sandbox: {error}"))?;
    command
        .status()
        .map_err(|error| format!("failed to execute in sandbox: {error}"))
}
