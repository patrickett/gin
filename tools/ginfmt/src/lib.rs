pub mod align;
pub mod align_ast;
pub mod config;
pub mod mods;
pub mod visitor;

pub use config::Config;
pub use visitor::FmtVisitor;

// TODO: change this to a trait that can be implemented for each ast node
// trait CodeFormat format()
use visitor::format as format_internal;

/// Format Gin source code using default configuration.
pub fn format(source: &str) -> String {
    format_internal(source, Config::default())
}

/// Format Gin source code using the provided configuration.
pub fn format_with_config(source: &str, config: Config) -> String {
    format_internal(source, config)
}
