use flask::FlaskConfig;
use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use std::path::PathBuf;

pub fn begin_build(
    config: FlaskConfig,
    input: Option<PathBuf>,
    timings: bool,
    release: bool,
    quiet: bool,
) -> bool {
    let (path, emit) = match input {
        Some(p) => (p, Emit::Exe),
        None => {
            let cwd = match std::env::current_dir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error: cannot get current directory: {e}");
                    return false;
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
        eprintln!("error: path does not exist: {}", path.display());
        return false;
    }

    // Resolve path dependencies relative to cwd (where mod.flask lives).
    let config_dir = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot get current directory: {e}");
            return false;
        }
    };
    let dependencies = config.resolve_path_dependencies(&config_dir);

    let mut args = Args {
        input: path,
        dependencies,
        emit,
        timings,
        profile: if release {
            Profile::Release
        } else {
            Profile::Debug
        },
        quiet,
        ..Default::default()
    };

    GinCompiler::compile_cli(&mut args).is_success()
}
