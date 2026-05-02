use clap::Args;
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use flask::PACKAGE_CONFIG_NAME;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Args, Debug, Clone)]
pub struct NewArgs {
    /// Name of the new project
    #[arg(value_name = "NAME")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum Template {
    HelloWorld,
    Library,
}

impl Template {
    fn variants() -> [&'static str; 2] {
        ["Hello World", "Library"]
    }
}

struct NewOptions {
    name: String,
    template: Template,
    author: String,
    git_init: bool,
}

/// `begin new [name]` creates a new Gin project in a new directory
pub fn begin_new(args: NewArgs) {
    let Some(options) = prompt_new_options(args.name) else {
        // User cancelled with Ctrl-C
        return;
    };

    let project_dir = PathBuf::from(&options.name);

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

    write_flask_json(&flask_path, &options.name, &options.author);

    let starter_gin = match options.template {
        Template::HelloWorld => project_dir.join(super::DEFAULT_ENTRY),
        Template::Library => project_dir.join(super::DEFAULT_LIB),
    };
    write_main_gin(&starter_gin, options.template);

    if options.git_init {
        run_git_init(&project_dir);
    }

    print_get_started(&options.name);
}

fn prompt_new_options(pre_filled_name: Option<String>) -> Option<NewOptions> {
    let theme = ColorfulTheme::default();

    let name = if let Some(n) = pre_filled_name {
        n
    } else {
        Input::with_theme(&theme)
            .with_prompt("Project name")
            .interact()
            .ok()?
    };

    let template_selection = Select::with_theme(&theme)
        .with_prompt("Template")
        .items(&Template::variants())
        .default(0)
        .interact()
        .ok()?;

    let template = match template_selection {
        0 => Template::HelloWorld,
        1 => Template::Library,
        _ => unreachable!(),
    };

    let author = Input::<String>::with_theme(&theme)
        .with_prompt("Author (leave blank to skip)")
        .allow_empty(true)
        .interact()
        .ok()?
        .trim()
        .to_string();

    let git_init = Confirm::with_theme(&theme)
        .with_prompt("Initialize a new git repository?")
        .default(true)
        .interact()
        .ok()?;

    Some(NewOptions {
        name,
        template,
        author,
        git_init,
    })
}

pub fn write_flask_json(path: &Path, name: &str, author: &str) {
    let authors_json = if author.is_empty() {
        "[]".to_string()
    } else {
        // Escape quotes in author name
        let escaped = author.replace('\\', "\\\\").replace('"', "\\\"");
        format!(r#"[\"{escaped}\"]"#)
    };

    let content = format!(
        r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "authors": {authors_json},
  "dependencies": {{}}
}}
"#
    );

    if let Err(e) = fs::write(path, content) {
        eprintln!("error: could not write `{}`: {e}", path.display());
    }
}

pub fn write_main_gin(path: &Path, template: Template) {
    let content = match template {
        Template::HelloWorld => {
            r#"main:
    print("Hello, world!")
return
"#
        }
        Template::Library => {
            r#"add(a: int, b: int) -> int:
    return a + b
"#
        }
    };

    if let Err(e) = fs::write(path, content) {
        eprintln!("error: could not write `{}`: {e}", path.display());
    }
}

fn run_git_init(dir: &Path) {
    match Command::new("git").arg("init").current_dir(dir).output() {
        Ok(_) => {
            println!("Initialized git repository");
        }
        Err(e) => {
            eprintln!("warning: failed to initialize git repository: {e}");
        }
    }
}

fn print_get_started(name: &str) {
    println!();
    println!("     Created `{name}` package");
    println!();
    println!("  Get started:");
    println!("    cd {name}");
    println!("    begin run");
}
