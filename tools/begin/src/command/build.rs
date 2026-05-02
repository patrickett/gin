use flask::FlaskConfig;
use ginc::cli::{Args, Emit};
use ginc::compile::GinCompiler;
use std::path::PathBuf;

// TODO: compiler performance, show time spend on io, and the number of syscalls

/// `begin (b)uild` will build
pub fn begin_build(config: FlaskConfig, input: Option<PathBuf>) {
    let (path, emit) = match input {
        Some(p) => (p, Emit::Exe),
        None => {
            let cwd = match std::env::current_dir() {
                Ok(d) => d,
                Err(_) => {
                    todo!("fancy error message for bad path")
                }
            };

            let main_gin = cwd.join(super::DEFAULT_ENTRY);
            if main_gin.is_file() {
                (main_gin, Emit::Exe)
            } else {
                (cwd, Emit::Obj)
            }
        }
    };

    if !path.exists() {
        todo!("fancy error message for path 404")
    }

    // Resolve path dependencies relative to cwd (where flask.jsonc lives).
    let config_dir = std::env::current_dir().unwrap_or_default();
    let dependencies = super::resolve_path_dependencies(&config, &config_dir);

    // For libraries, use package name as output
    let output = if emit == Emit::Obj {
        let pkg_name = format!("{}.o", config.name.replace('-', "_"));
        Some(config_dir.join("target").join(pkg_name))
    } else {
        None
    };

    let mut args = Args {
        input: path,
        dependencies,
        emit,
        output,
        ..Default::default()
    };

    GinCompiler::compile(&mut args)
}
