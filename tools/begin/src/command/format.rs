use flask::FlaskConfig;
use ignore::WalkBuilder;
use std::{fs, path::Path};

pub fn begin_format() -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    format_project(&cwd)
}

fn format_project(from: &Path) -> Result<(), String> {
    let (_, root) = FlaskConfig::discover(from)?
        .ok_or_else(|| format!("no package manifest found in or above {}", from.display()))?;
    let entries = WalkBuilder::new(&root)
        .require_git(false)
        .filter_entry(|entry| {
            entry.depth() == 0
                || !(entry.file_type().is_some_and(|kind| kind.is_dir())
                    && FlaskConfig::manifest_path(&entry.path()).is_file())
        })
        .build();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if !entry.file_type().is_some_and(|kind| kind.is_file())
            || !path.extension().is_some_and(|extension| extension == "gin")
        {
            continue;
        }
        let source =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let formatted = ginfmt::format(&source);
        if formatted != source {
            fs::write(path, formatted).map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BeginArguments, command::BeginCommand};
    use clap::Parser;
    use flask::MANIFEST_NAME;

    #[test]
    fn parses_format_command_and_alias() {
        for name in ["format", "fmt"] {
            let args = BeginArguments::try_parse_from(["begin", name]).unwrap();
            assert!(matches!(args.command, BeginCommand::Format));
        }
    }

    #[test]
    fn formats_package_from_subdirectory_and_leaves_excluded_files_unchanged() {
        let root = test_fixtures::unique_temp_dir("begin-format");
        for dir in ["src", "target", "dependency", "ignored"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let config = r#"#settings
module: "format-fixture"
version: 0.1.0

#dependencies
"#;
        fs::write(root.join(MANIFEST_NAME), config).unwrap();
        fs::write(root.join("dependency").join(MANIFEST_NAME), config).unwrap();
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        let source = "value:1\n";
        let included = ["main.gin", "src/value.gin", "target/value.gin"];
        let excluded = ["dependency/value.gin", "ignored/value.gin", "notes.txt"];
        for path in included.iter().chain(&excluded) {
            fs::write(root.join(path), source).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("notes.txt"), root.join("link.gin")).unwrap();

        format_project(&root.join("src")).unwrap();
        for path in included {
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), "value: 1\n");
        }
        for path in excluded {
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), source);
        }
        format_project(&root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("main.gin")).unwrap(),
            "value: 1\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
