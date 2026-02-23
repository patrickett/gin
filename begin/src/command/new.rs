use clap::Args;
use flask::PACKAGE_CONFIG_NAME;
use ginc::BINARY_ENTRY_FILE_NAME;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Args, Debug)]
pub struct NewArgs {
    /// Name of the new project
    #[arg(value_name = "NAME")]
    pub name: String,
}

/// `begin new <name>` creates a new Gin project in a new directory
pub fn begin_new(args: NewArgs) {
    let project_dir = PathBuf::from(&args.name);

    if project_dir.exists() {
        eprintln!(
            "error: destination `{}` already exists",
            project_dir.display()
        );
        return;
    }

    if let Err(e) = fs::create_dir_all(&project_dir) {
        eprintln!(
            "error: could not create directory `{}`: {e}",
            project_dir.display()
        );
        return;
    }

    let flask_path = project_dir.join(PACKAGE_CONFIG_NAME);
    let main_path = project_dir.join(BINARY_ENTRY_FILE_NAME);

    write_flask_json(&flask_path, &args.name);
    write_main_gin(&main_path);

    println!("created `{}` package", args.name);
}

pub fn write_flask_json(path: &Path, name: &str) {
    let content = format!(
        r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "authors": [],
  "dependencies": {{}}
}}
"#
    );

    if let Err(e) = fs::write(path, content) {
        eprintln!("error: could not write `{}`: {e}", path.display());
    }
}

pub fn write_main_gin(path: &Path) {
    let content = r#"main:
    print("Hello, world!")
return
"#;

    if let Err(e) = fs::write(path, content) {
        eprintln!("error: could not write `{}`: {e}", path.display());
    }
}
