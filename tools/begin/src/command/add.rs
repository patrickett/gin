use clap::Parser;
use flask::{Dependency, DependencyKind, FlaskConfig};
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct AddArgs {
    /// Package to add (name@version for registry, path for local, URL for git)
    #[arg(value_name = "PACKAGE")]
    package: String,
}

/// `begin add` adds a dependency to the current project
pub fn begin_add(_config: FlaskConfig, args: AddArgs) {
    let package = args.package;

    let dependency = parse_package(&package);

    let dep_name = extract_package_name(&dependency);

    #[cfg(debug_assertions)]
    println!(
        "info: adding dependency `{dep_name}` from {source}",
        source = match &dependency.kind {
            DependencyKind::Version { version } => format!("registry version {version}"),
            DependencyKind::Path { path } => format!("local path {path}"),
            DependencyKind::Git { url } => format!("git repository {url}"),
        }
    );

    // TODO: Add dependency to flask.jsonc
    // 1. Find flask.jsonc in current directory or parent directories
    // 2. Parse existing config
    // 3. Add new dependency to dependencies map
    // 4. Write updated config back to flask.jsonc

    todo!("Add dependency to flask.jsonc: {dep_name}")
}

fn parse_package(package: &str) -> Dependency {
    if package.contains('@') && !package.contains("://") {
        let parts: Vec<&str> = package.split('@').collect();
        let _name = parts[0];
        let version = parts[1];

        Dependency {
            kind: DependencyKind::Version {
                version: version.to_string(),
            },
            common: Default::default(),
        }
    } else if package.contains("://") {
        Dependency {
            kind: DependencyKind::Git {
                url: package.to_string(),
            },
            common: Default::default(),
        }
    } else {
        let _path = PathBuf::from(package);
        Dependency {
            kind: DependencyKind::Path {
                path: package.to_string(),
            },
            common: Default::default(),
        }
    }
}

fn extract_package_name(dependency: &Dependency) -> String {
    match &dependency.kind {
        DependencyKind::Version { version } => {
            // For registry packages, we'd need to query the package name
            format!("package@{version}")
        }
        DependencyKind::Path { path } => {
            let path = PathBuf::from(path);
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        }
        DependencyKind::Git { url } => {
            let name_part = url.rsplit('/').next().unwrap_or(url);
            name_part.replace(".git", "")
        }
    }
}
