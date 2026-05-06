use crate::command::{
    add::{AddArgs, begin_add},
    audit::begin_audit,
    build::begin_build,
    doc::{DocCommand, begin_doc},
    init::begin_init,
    new::{NewArgs, begin_new},
    run::begin_run,
    version::{VersionCommand, version},
};
use clap::*;
use flask::{DependencyKind, FlaskConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_ENTRY: &str = "main.gin";
pub const DEFAULT_LIB: &str = "lib.gin";

/// Resolve path dependencies relative to a config directory.
///
/// Takes the FlaskConfig dependencies and resolves all `Path` dependencies
/// to absolute paths based on the provided config directory.
pub fn resolve_path_dependencies(
    config: &FlaskConfig,
    config_dir: &Path,
) -> HashMap<String, PathBuf> {
    let mut dependencies = HashMap::new();
    for (name, dep) in config.dependencies() {
        if let DependencyKind::Path { path: dep_path } = &dep.kind {
            dependencies.insert(name.clone(), config_dir.join(dep_path));
        }
    }
    dependencies
}

mod add;
mod audit;
mod build;
mod doc;
mod init;
mod new;
mod run;
mod version;

#[derive(Subcommand, Debug)]
pub enum BeginCommand {
    // begin add pkg_name
    // begin add "*.git" for git
    Add,

    /// Runs a security audit on the dependencies of the current project
    Audit,

    /// Run the benchmarks declared in the current project
    Bench,

    /// Compile the specified module (Default: cwd)
    #[command(alias = "b")]
    Build {
        path: Option<PathBuf>,
        watch: Option<bool>,
    },

    /// Analyze the current module and report errors, but don't build object files
    #[command(alias = "c")]
    Check {},

    /// Generate documentation for current package and its dependencies
    #[command(subcommand, alias = "d")]
    Doc(DocCommand),

    /// Format all the tracked files in the current package with 'ginfmt'
    Format,

    /// Initialise a new Gin project in the current directory
    Init,

    /// Create a new Gin project in a new directory
    New(NewArgs),

    /// Run the current project, will just compile a library if no entry
    #[command(alias = "r")]
    Run {
        path: Option<PathBuf>,
        watch: Option<bool>,
    },

    /// Run the tests declared in the current project
    Test,

    #[command(subcommand, alias = "v")]
    Version(VersionCommand),
    // TODO: begin info {pkg}
    //     - list info for the package
    //     - optionally show dependencies
    //     - https://docs.deno.com/runtime/reference/cli/info/
    //     - show cache location
    // Info,
}

impl BeginCommand {
    /// Returns true if this command can run without an existing flask.jsonc
    pub fn needs_config(&self) -> bool {
        !matches!(self, BeginCommand::Init | BeginCommand::New(_))
    }

    pub fn run(&self, config: Option<FlaskConfig>) {
        match &self {
            BeginCommand::Init => begin_init(),
            BeginCommand::New(args) => begin_new(args.clone()),
            _ => {
                let Some(config) = config else {
                    return;
                };
                self.run_with_config(config);
            }
        }
    }

    fn run_with_config(&self, config: FlaskConfig) {
        match &self {
            BeginCommand::Add => {
                let args = AddArgs::parse_from(std::env::args());
                begin_add(config, args)
            }
            BeginCommand::Audit => begin_audit(config),
            BeginCommand::Build { path: input, .. } => begin_build(config, input.to_owned()),
            BeginCommand::Doc(_cmd) => begin_doc(config),
            BeginCommand::Run { path: input, watch } => {
                begin_run(config, input.to_owned(), watch.unwrap_or(false))
            }
            BeginCommand::Version(cmd) => version(cmd),
            _ => eprintln!("warning: command not yet implemented"),
        }
    }
}
