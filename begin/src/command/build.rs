use flask::{DependencyKind, FlaskConfig};
use ginc::{Args, GinCompiler};
use std::collections::HashMap;
use std::path::PathBuf;

// TODO: compiler performance, show time spend on io, and the number of syscalls

/// `begin (b)uild` will build
pub fn begin_build(config: FlaskConfig, input: Option<PathBuf>) {
    let path = match input {
        Some(p) => p,
        None => {
            let cwd = match std::env::current_dir() {
                Ok(d) => d,
                Err(_) => {
                    todo!("fancy error message for bad path")
                }
            };

            let Some(entry) = config.entry() else {
                eprintln!("Error: no \"entry\" field in flask.json");
                return;
            };

            cwd.join(entry)
        }
    };

    if !path.exists() {
        todo!("fancy error message for path 404")
    }

    // Resolve path dependencies relative to cwd (where flask.json lives).
    let config_dir = std::env::current_dir().unwrap_or_default();
    let mut dependencies: HashMap<String, PathBuf> = HashMap::new();
    for (name, dep) in config.dependencies() {
        if let DependencyKind::Path { path: dep_path } = &dep.kind {
            dependencies.insert(name.clone(), config_dir.join(dep_path));
        }
    }

    let mut args = Args {
        input: path,
        dependencies,
        ..Default::default()
    };

    GinCompiler::compile(&mut args)
}
