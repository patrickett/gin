use flask::MANIFEST_NAME;
use std::{env, path::PathBuf};

use crate::command::new::{write_main_gin, write_manifest};

/// `begin init` initialises a Gin project in the current directory
pub fn begin_init() {
    let cwd = match env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return;
        }
    };

    let flask_path = cwd.join(MANIFEST_NAME);

    if flask_path.exists() {
        eprintln!(
            "error: `{MANIFEST_NAME}` already exists in `{}`",
            cwd.display()
        );
        return;
    }

    let project_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();

    write_manifest(&flask_path, &project_name, "");

    let main_path = PathBuf::from(super::DEFAULT_ENTRY);
    if !main_path.exists() {
        write_main_gin(&main_path, crate::command::new::Template::HelloWorld);
    }

    println!("initialised `{project_name}` package");
}
