use flask::PACKAGE_CONFIG_NAME;
use ginc::BINARY_ENTRY_FILE_NAME;
use std::{env, path::PathBuf};

use crate::command::new::{write_flask_json, write_main_gin};

/// `begin init` initialises a Gin project in the current directory
pub fn begin_init() {
    let cwd = match env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return;
        }
    };

    let flask_path = cwd.join(PACKAGE_CONFIG_NAME);

    if flask_path.exists() {
        eprintln!(
            "error: `{PACKAGE_CONFIG_NAME}` already exists in `{}`",
            cwd.display()
        );
        return;
    }

    let project_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();

    write_flask_json(&flask_path, &project_name);

    let main_path = PathBuf::from(BINARY_ENTRY_FILE_NAME);
    if !main_path.exists() {
        write_main_gin(&main_path);
    }

    println!("initialised `{project_name}` package");
}
