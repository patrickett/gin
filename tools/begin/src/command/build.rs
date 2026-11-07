use flask::FlaskConfig;
use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use std::path::PathBuf;

pub fn begin_build(
    _config: FlaskConfig,
    input: Option<PathBuf>,
    timings: bool,
    release: bool,
    quiet: bool,
) -> bool {
    let path = match input {
        Some(path) => path,
        None => {
            let cwd = match std::env::current_dir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error: cannot get current directory: {e}");
                    return false;
                }
            };

            FlaskConfig::find_package_config(&cwd)
                .map(|(_, root)| root)
                .unwrap_or(cwd)
        }
    };

    if !path.exists() {
        eprintln!("error: path does not exist: {}", path.display());
        return false;
    }

    let mut args = Args {
        input: path,
        emit: Emit::Auto,
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
