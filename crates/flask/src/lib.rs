#![deny(unsafe_code)]
#![warn(
    clippy::correctness,
    clippy::suspicious,
    clippy::style,
    clippy::complexity,
    clippy::perf
)]
mod compile_target;
mod config;
mod handle;
mod resolve;

pub use compile_target::{CompileTarget, TargetError, TargetTriple};
pub use config::{Dependency, DependencyKind, FlaskConfig, PACKAGE_CONFIG_NAME};
pub use handle::{ConfigError, FlaskConfigHandle, FlaskConfigReadGuard};
pub use resolve::{
    NestedPackageResolveError, NestedPackageTarget, list_package_gin_files,
    resolve_nested_package_path,
};
