use crate::command::{
    add::{AddArgs, begin_add},
    audit::begin_audit,
    build::begin_build,
    doc::{DocCommand, begin_doc},
    format::begin_format,
    init::begin_init,
    new::{NewArgs, begin_new},
    run::begin_run,
    version::{VersionCommand, version},
};
use clap::*;
use flask::{DependencyKind, FlaskConfig};
use ginc::reporting::{CheckOptions, MessageFormat};
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
mod check;
mod doc;
mod format;
mod init;
mod new;
mod registry;
mod run;
mod version;

#[derive(Subcommand, Debug)]
pub enum BeginCommand {
    /// Resolve eligible releases, fetch verified sources, and update flask.jsonc.
    Update,
    /// Fetch the exact manifest resolution without changing versions.
    Fetch {
        /// Use unexpired cached metadata and cached archives only.
        #[arg(long)]
        offline: bool,
    },
    // begin add pkg_name
    // begin add "*.git" for git
    Add,

    /// Runs a security audit on the dependencies of the current project
    Audit,

    /// Compile the specified module (Default: cwd)
    #[command(alias = "b")]
    Build {
        path: Option<PathBuf>,
        watch: Option<bool>,
        /// Show compilation phase durations and cache outcome.
        #[arg(long)]
        timings: bool,
        /// Build optimized code.
        #[arg(long)]
        release: bool,
        /// Suppress build status messages.
        #[arg(short, long)]
        quiet: bool,
    },

    /// Validate Gin sources without generating code; exits nonzero on errors.
    #[command(alias = "c")]
    Check {
        path: Option<PathBuf>,
        #[arg(long)]
        target: Option<String>,
        /// JSON emits one object per line on stdout; human and short use stderr.
        #[arg(long, value_enum, default_value = "human")]
        message_format: MessageFormat,
        /// Limit displayed errors while still checking all sources.
        #[arg(long)]
        max_errors: Option<usize>,
    },

    /// Generate documentation for current package and its dependencies
    #[command(subcommand, alias = "d")]
    Doc(DocCommand),

    /// Format project Gin files in place
    #[command(alias = "fmt")]
    Format,

    /// Initialise a new Gin project in the current directory
    Init,

    /// Create a new Gin project in a new directory
    New(NewArgs),

    /// Compile and run with isolated storage and no network unless explicitly granted.
    #[command(alias = "r")]
    Run {
        path: Option<PathBuf>,
        watch: Option<bool>,
        /// Grant read access to an existing file or directory.
        #[arg(long, value_name = "PATH")]
        allow_read: Vec<PathBuf>,
        /// Grant read and write access to an existing file or directory.
        #[arg(long, value_name = "PATH")]
        allow_write: Vec<PathBuf>,
        /// Grant network access to the launched program.
        #[arg(long)]
        allow_network: bool,
    },

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
    pub fn needs_preloaded_config(&self) -> bool {
        !matches!(
            self,
            BeginCommand::Check { .. }
                | BeginCommand::Init
                | BeginCommand::New(_)
                | BeginCommand::Format
                | BeginCommand::Version(_)
                | BeginCommand::Update
                | BeginCommand::Fetch { .. }
        )
    }

    pub fn run(&self, config: Option<FlaskConfig>) {
        match &self {
            BeginCommand::Check {
                path,
                target,
                message_format,
                max_errors,
            } => {
                let options = CheckOptions {
                    message_format: *message_format,
                    max_errors: *max_errors,
                };
                match check::begin_check(path.as_deref(), target.as_deref(), options) {
                    Ok(true) => {}
                    Ok(false) => std::process::exit(1),
                    Err(error) => {
                        if *message_format == MessageFormat::Json {
                            println!(
                                "{}",
                                serde_json::json!({"reason": "diagnostic", "severity": "error", "code": "check-failed", "message": error})
                            );
                            println!(
                                "{}",
                                serde_json::json!({"reason": "check-finished", "success": false, "errors": 1, "warnings": 0, "omitted_errors": 0})
                            );
                        } else {
                            eprintln!("begin check: {error}");
                        }
                        std::process::exit(1);
                    }
                }
            }
            BeginCommand::Init => begin_init(),
            BeginCommand::New(args) => begin_new(args.clone()),
            BeginCommand::Version(cmd) => version(cmd),
            BeginCommand::Update | BeginCommand::Fetch { .. } => {
                let update = matches!(self, BeginCommand::Update);
                let offline = matches!(self, BeginCommand::Fetch { offline: true });
                if let Err(error) = registry::registry(update, offline) {
                    eprintln!("begin: {error}");
                    std::process::exit(1);
                }
            }
            BeginCommand::Format => {
                if let Err(error) = begin_format() {
                    eprintln!("begin format: {error}");
                    std::process::exit(1);
                }
            }
            _ => {
                let Some(config) = config else {
                    if matches!(self, BeginCommand::Build { .. }) {
                        eprintln!("error: could not find flask.jsonc for this build");
                        std::process::exit(1);
                    }
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
            BeginCommand::Build {
                path: input,
                timings,
                release,
                quiet,
                ..
            } => {
                if !begin_build(config, input.to_owned(), *timings, *release, *quiet) {
                    std::process::exit(1);
                }
            }
            BeginCommand::Doc(_cmd) => begin_doc(config),
            BeginCommand::Run {
                path: input,
                watch,
                allow_read,
                allow_write,
                allow_network,
            } => {
                let mut sandbox = flask::ExecutionSandbox::default().with_network(*allow_network);
                for path in allow_read {
                    sandbox = sandbox.with_read(path.clone());
                }
                for path in allow_write {
                    sandbox = sandbox.with_write(path.clone());
                }
                begin_run(config, input.to_owned(), watch.unwrap_or(false), sandbox)
            }
            BeginCommand::Check { .. }
            | BeginCommand::Init
            | BeginCommand::New(_)
            | BeginCommand::Format
            | BeginCommand::Version(_)
            | BeginCommand::Update
            | BeginCommand::Fetch { .. } => {
                unreachable!("commands without preloaded config are dispatched in `run`")
            }
        }
    }
}
