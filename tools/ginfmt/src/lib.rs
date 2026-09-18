pub mod align_ast;
pub mod ast_formatter;
pub mod config;

pub use ast_formatter::AstFormatter;
pub use config::Config;
use diagnostic::Category;
use parser::query::SourceParseExt;

/// Format Gin source code using the Gin AST parser.
///
/// Parses the source into an `ast::FileAst`, walks it to build formatted
/// output.
pub fn format(source: &str) -> String {
    format_with_config(source, Config::default())
}

/// Format Gin source code using the provided configuration.
pub fn format_with_config(source: &str, config: Config) -> String {
    let output = source.parse_source_full();
    if output
        .symptoms
        .iter()
        .any(|symptom| symptom.category == Category::Flaw)
        || output.comments.iter().any(|comment| !comment.top_level)
    {
        return source.to_string();
    }
    let mut formatter =
        AstFormatter::new(source, &config, &output.ast.span_table).with_comments(&output.comments);
    let formatted = formatter.format_file(&output.ast);
    let reparsed = formatted.parse_source_full();
    if reparsed
        .symptoms
        .iter()
        .any(|symptom| symptom.category == Category::Flaw)
        || !output
            .comments
            .iter()
            .map(|comment| &comment.text)
            .eq(reparsed.comments.iter().map(|comment| &comment.text))
    {
        return source.to_string();
    }
    formatted
}
