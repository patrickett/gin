use ginc::cli::Args;
use ginc::compile::GinCompiler;
use ginc::reporting::CheckOptions;
use std::path::Path;

pub fn begin_check(
    path: Option<&Path>,
    target: Option<&str>,
    options: CheckOptions,
) -> Result<bool, String> {
    let input = match path {
        Some(path) => path.to_path_buf(),
        None => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            flask::FlaskConfig::find_package_config(&cwd)
                .map(|(_, root)| root)
                .unwrap_or(cwd)
        }
    };
    let mut args = Args {
        input,
        target: target.map(str::to_owned),
        ..Args::default()
    };
    Ok(GinCompiler::check(&mut args, options).is_success())
}
