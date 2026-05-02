use flask::FlaskConfig;
use ginc::cli::{Args, Emit};
use ginc::compile::GinCompiler;
use std::path::PathBuf;
use std::process::Command;

// TODO: support passing args to the executable: begin run -- --some-arg

/// `begin (r)un` compiles and executes the default entry file (see [`super::DEFAULT_ENTRY`])
/// or the given file. Without that entry, treats the package as a library and does not run.
pub fn begin_run(config: FlaskConfig, input: Option<PathBuf>, watch: bool) {
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

    // Determine executable path (same as input but without extension)
    let exe_path = path.with_extension("");

    // Compile to executable
    eprintln!("Compiling {} v{}", config.name, config.version);

    let mut args = Args {
        input: path.clone(),
        dependencies: dependencies.clone(),
        emit: Emit::Exe,
        output: Some(exe_path.clone()),
        ..Default::default()
    };

    GinCompiler::compile(&mut args);

    // Check if compilation succeeded
    if !exe_path.exists() {
        eprintln!("error: Compilation failed");
        return;
    }

    eprintln!("Running `{}`", exe_path.display());

    let mut cmd = Command::new(&exe_path);
    // TODO: forward args to executable when args passthrough is implemented
    let status = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: Failed to execute: {}", e);
            return;
        }
    };

    if !status.success() {
        eprintln!("info: Exited with status: {}", status);
    }

    // TODO: implement watch mode for recompile+run loop
    if watch {
        eprintln!("warning: Watch mode not yet implemented - run once");
    }
}
