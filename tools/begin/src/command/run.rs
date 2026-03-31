use flask::{DependencyKind, FlaskConfig};
use ginc::cli::{Args, Emit};
use ginc::compile::GinCompiler;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

// TODO: support passing args to the executable: begin run -- --some-arg

/// `begin (r)un` compiles and executes the entry point.
/// For libraries, prints a helpful message suggesting `begin build`.
pub fn begin_run(config: FlaskConfig, input: Option<PathBuf>, watch: bool) {
    // Check if this is a binary (has entry) or a library
    let Some(entry) = config.entry() else {
        eprintln!(
            "warning: '{}' is a library package. Use `begin build` to compile libraries.",
            config.name
        );
        return;
    };

    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => {
            eprintln!("error: Failed to get current directory");
            return;
        }
    };

    let path = input.unwrap_or_else(|| cwd.join(entry));

    if !path.exists() {
        eprintln!("error: Entry file not found: {}", path.display());
        return;
    }

    // Resolve path dependencies relative to cwd (where flask.json lives).
    let config_dir = std::env::current_dir().unwrap_or_default();
    let mut dependencies: HashMap<String, PathBuf> = HashMap::new();
    for (name, dep) in config.dependencies() {
        if let DependencyKind::Path { path: dep_path } = &dep.kind {
            dependencies.insert(name.clone(), config_dir.join(dep_path));
        }
    }

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
