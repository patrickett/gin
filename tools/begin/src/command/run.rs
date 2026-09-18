use flask::FlaskConfig;
use ginc::cli::{Args, Emit};
use ginc::compile::{CompileResult, GinCompiler};
use std::path::PathBuf;

// TODO: support passing args to the executable: begin run -- --some-arg

/// `begin (r)un` compiles and executes the default entry file (see [`super::DEFAULT_ENTRY`])
/// or the given file. Without that entry, treats the package as a library and does not run.
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

    let main_path = cwd.join(super::DEFAULT_ENTRY);
    let path = match input {
        Some(p) => p,
        None if main_path.is_file() => main_path,
        None => {
            eprintln!(
                "warning: '{}' has no {} — library package. Use `begin build` for libraries.",
                config.name,
                super::DEFAULT_ENTRY
            );
            return;
        }
    };

    if !path.exists() {
        eprintln!("error: Entry file not found: {}", path.display());
        return;
    }

    // Resolve path dependencies relative to cwd (where flask.jsonc lives).
    let config_dir = std::env::current_dir().unwrap_or_default();
    let dependencies = super::resolve_path_dependencies(&config, &config_dir);

    eprintln!("Compiling {} v{}", config.name, config.version);
    let mut args = Args {
        input: path.clone(),
        dependencies,
        emit: Emit::Exe,
        ..Default::default()
    };
    let exe_path = match GinCompiler::compile(&mut args) {
        CompileResult::Success { output: Some(path) }
        | CompileResult::SuccessWithTypecheckDiagnostics { output: Some(path) } => path,
        _ => {
            eprintln!("error: Compilation failed");
            std::process::exit(1);
        }
    };

    eprintln!("Running `{}`", exe_path.display());

    let scratch = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("error: cannot create execution sandbox: {error}");
            std::process::exit(1);
        }
    };
    let mut cmd = match sandbox.command(&exe_path, scratch.path()) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: cannot configure execution sandbox: {error}");
            std::process::exit(1);
        }
    };
    // TODO: forward args to executable when args passthrough is implemented
    let status = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: Failed to execute in sandbox: {}", e);
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
