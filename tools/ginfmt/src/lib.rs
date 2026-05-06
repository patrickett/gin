#![deny(unsafe_code)]
#![warn(clippy::correctness, clippy::suspicious, clippy::style, clippy::complexity, clippy::perf)]
pub mod align_ast;
pub mod ast_formatter;
pub mod config;

pub use ast_formatter::AstFormatter;
pub use config::Config;

/// Format Gin source code using the Gin AST parser.
///
/// Parses the source into an `ast::FileAst`, walks it to build formatted
/// output, then applies alignment and line wrapping.
pub fn format(source: &str) -> String {
    format_with_config(source, Config::default())
}

/// Format Gin source code using the provided configuration.
pub fn format_with_config(source: &str, config: Config) -> String {
    let output = parser::parse_source_full(source);
    let mut formatter = AstFormatter::new(source, &config, output.ast.span_table());
    formatter.format_file(&output.ast)
}
